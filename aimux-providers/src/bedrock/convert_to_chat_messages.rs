//! Conversion between `LanguageModelPrompt` and Amazon Bedrock Converse format.
//!
//! Mirrors `convert-to-amazon-bedrock-chat-messages.ts` in the TS SDK. The
//! Converse API uses a unified message format across all model providers:
//!
//! - System messages are lifted out of `messages` into a top-level `system`
//!   array of `{ "text": "..." }` blocks.
//! - User and tool messages are merged into `role: "user"` messages.
//! - Assistant messages become `role: "assistant"`.
//! - Tool calls become `toolUse` blocks; tool results become `toolResult`
//!   blocks.
//! - Consecutive same-role messages are merged into a single message (matching
//!   the TS `groupIntoBlocks` behaviour).

use super::normalize_tool_call_id::normalize_tool_call_id;
use super::options;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_provider_utils::is_full_media_type;
use base64::Engine;
use serde_json::{Value, json};

// ── Prompt conversion ───────────────────────────────────────────────────────

/// Convert a provider-facing prompt into Bedrock's `{ system, messages }` shape.
///
/// Returns `(system: Vec<Value>, messages: Vec<Value>)`.
///
/// Mirrors the TS `convertToAmazonBedrockChatMessages`: consecutive user+tool
/// messages are grouped into a single `user` block, consecutive assistant
/// messages into a single `assistant` block. Assistant text is trimmed when it
/// is the last content part of the last message of the last block; empty
/// assistant text is dropped unless the message also carries reasoning.
///
/// # Errors
///
/// Returns an error for prompt content the Converse API cannot express.
#[cfg(test)]
pub(super) fn convert_prompt_to_bedrock(
    prompt: &LanguageModelPrompt,
) -> Result<(Vec<Value>, Vec<Value>), aimux_core::AiMuxError> {
    convert_prompt(prompt, false)
}

pub(super) fn convert_prompt(
    prompt: &LanguageModelPrompt,
    is_mistral: bool,
) -> Result<(Vec<Value>, Vec<Value>), aimux_core::AiMuxError> {
    validate_prompt(prompt)?;
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Blk {
        System,
        User,
        Assistant,
    }

    // Group consecutive same-block messages (user+tool fold into User).
    let mut blocks: Vec<(Blk, Vec<usize>)> = Vec::new();
    for (i, msg) in prompt.iter().enumerate() {
        let b = match msg {
            LanguageModelMessage::System { .. } => Blk::System,
            LanguageModelMessage::User { .. } | LanguageModelMessage::Tool { .. } => Blk::User,
            LanguageModelMessage::Assistant { .. } => Blk::Assistant,
        };
        if blocks.last().map(|(lb, _)| *lb) == Some(b) {
            blocks.last_mut().unwrap().1.push(i);
        } else {
            blocks.push((b, vec![i]));
        }
    }

    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut document_counter: u32 = 0;
    let num_blocks = blocks.len();

    for (bi, (blk, idxs)) in blocks.iter().enumerate() {
        let is_last_block = bi == num_blocks - 1;
        match blk {
            Blk::System => {
                for &i in idxs {
                    let LanguageModelMessage::System {
                        content,
                        provider_options,
                    } = &prompt[i]
                    else {
                        unreachable!()
                    };
                    system.push(json!({ "text": content }));
                    if let Some(cp) = cache_point(provider_options.as_ref()) {
                        system.push(cp);
                    }
                }
            }
            Blk::User => {
                let mut content: Vec<Value> = Vec::new();
                for &i in idxs {
                    let provider_options = match &prompt[i] {
                        LanguageModelMessage::User {
                            content: parts,
                            provider_options,
                        } => {
                            for part in parts {
                                let po = match part {
                                    UserPart::Text(part) => {
                                        push_user_text(part, &mut content);
                                        &part.provider_options
                                    }
                                    UserPart::File(file) => {
                                        if matches!(file.data, FileData::Reference { .. }) {
                                            continue;
                                        }
                                        push_file_part(file, &mut content, &mut document_counter);
                                        &file.provider_options
                                    }
                                };
                                if let Some(cp) = cache_point(po.as_ref()) {
                                    content.push(cp);
                                }
                            }
                            provider_options
                        }
                        LanguageModelMessage::Tool {
                            content: parts,
                            provider_options,
                        } => {
                            for part in parts {
                                let ToolPart::ToolResult(part) = part else {
                                    continue;
                                };
                                push_tool_result(
                                    part,
                                    &mut content,
                                    &mut document_counter,
                                    is_mistral,
                                );
                                if let Some(cp) = cache_point(part.provider_options.as_ref()) {
                                    content.push(cp);
                                }
                            }
                            provider_options
                        }
                        _ => unreachable!(),
                    };
                    if let Some(cp) = cache_point(provider_options.as_ref()) {
                        content.push(cp);
                    }
                }
                append_user_message(&mut messages, content);
            }
            Blk::Assistant => {
                let mut content: Vec<Value> = Vec::new();
                let mut results: Vec<Value> = Vec::new();
                let num_msgs = idxs.len();
                for (mj, &i) in idxs.iter().enumerate() {
                    let is_last_message = mj == num_msgs - 1;
                    let LanguageModelMessage::Assistant {
                        content: parts,
                        provider_options,
                    } = &prompt[i]
                    else {
                        unreachable!()
                    };
                    let has_reasoning = parts
                        .iter()
                        .any(|p| matches!(p, AssistantPart::Reasoning(_)));
                    let num_parts = parts.len();
                    for (kj, p) in parts.iter().enumerate() {
                        let is_last_content_part = kj == num_parts - 1;
                        if !matches!(p, AssistantPart::ToolResult(_)) && !results.is_empty() {
                            append_user_message(&mut messages, std::mem::take(&mut results));
                        }
                        match p {
                            AssistantPart::Text(TextPart { text, .. }) => {
                                // Skip empty text unless the message has reasoning.
                                if !text.trim().is_empty() || has_reasoning {
                                    let t =
                                        if is_last_block && is_last_message && is_last_content_part
                                        {
                                            text.trim().to_string()
                                        } else {
                                            text.clone()
                                        };
                                    content.push(json!({ "text": t }));
                                }
                            }
                            AssistantPart::Reasoning(ReasoningPart {
                                text,
                                provider_options,
                            }) => {
                                let metadata = options::read(provider_options.as_ref());
                                if let Some(sig) = metadata
                                    .and_then(|v| v.get("signature"))
                                    .and_then(Value::as_str)
                                {
                                    content.push(json!({"reasoningContent":{"reasoningText":{"text":text,"signature":sig}}}));
                                } else if let Some(redacted) =
                                    metadata.and_then(|v| v.get("redactedContent"))
                                {
                                    content.push(
                                        json!({"reasoningContent":{"redactedContent":redacted}}),
                                    );
                                } else if let Some(data) =
                                    metadata.and_then(|v| v.get("redactedData"))
                                {
                                    content.push(json!({"reasoningContent":{"redactedReasoning":{"data":data}}}));
                                }
                            }
                            AssistantPart::ToolResult(part) => {
                                flush_assistant(&mut messages, &mut content);
                                push_tool_result(
                                    part,
                                    &mut results,
                                    &mut document_counter,
                                    is_mistral,
                                );
                            }
                            AssistantPart::ToolCall(ToolCallPart {
                                tool_call_id,
                                tool_name,
                                input,
                                ..
                            }) => {
                                let input_val = if input.is_object() {
                                    input.clone()
                                } else {
                                    json!({ "rawInvalidInput": input })
                                };
                                content.push(json!({
                                    "toolUse": {
                                        "toolUseId": normalize_tool_call_id(tool_call_id, is_mistral),
                                        "name": sanitize_tool_name(tool_name),
                                        "input": input_val,
                                    }
                                }));
                            }
                            _ => {}
                        }
                        if let Some(cp) = assistant_cache_point(p) {
                            if matches!(p, AssistantPart::ToolResult(_)) {
                                results.push(cp);
                            } else {
                                content.push(cp);
                            }
                        }
                    }
                    if let Some(cp) = cache_point(provider_options.as_ref()) {
                        if matches!(parts.last(), Some(AssistantPart::ToolResult(_))) {
                            results.push(cp);
                        } else {
                            content.push(cp);
                        }
                    }
                }
                if !results.is_empty() {
                    append_user_message(&mut messages, results);
                }
                flush_assistant(&mut messages, &mut content);
            }
        }
    }

    Ok((system, messages))
}

fn push_user_text(part: &TextPart, content: &mut Vec<Value>) {
    let opts = options::read(part.provider_options.as_ref());
    if opts
        .and_then(|v| v.get("guardContent"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        let mut block = json!({"text":part.text});
        if let Some(q) = opts.and_then(|v| v.get("guardContentQualifiers")) {
            block["qualifiers"] = q.clone();
        }
        content.push(json!({"guardContent":{"text":block}}));
    } else {
        content.push(json!({ "text": part.text }));
    }
}

fn push_tool_result(
    part: &ToolResultPart,
    content: &mut Vec<Value>,
    doc_counter: &mut u32,
    is_mistral: bool,
) {
    let result_content = resolve_tool_result_output(&part.output, doc_counter);
    content.push(json!({
        "toolResult": {
            "toolUseId": normalize_tool_call_id(&part.tool_call_id, is_mistral),
            "content": result_content,
        }
    }));
}

fn push_file_part(file: &FilePart, content: &mut Vec<Value>, doc_counter: &mut u32) {
    let b64 = match &file.data {
        FileData::Data {
            data: FileBytes::Binary(bytes),
        } => base64::engine::general_purpose::STANDARD.encode(bytes),
        FileData::Data {
            data: FileBytes::Base64(data),
        } => data.clone(),
        FileData::Text { text } => base64::engine::general_purpose::STANDARD.encode(text),
        FileData::Url { .. } => String::new(),
        FileData::Reference { .. } => return,
    };
    let resolved_media_type = aimux_provider_utils::resolve_full_media_type(file).ok();
    let media_type =
        if matches!(file.data, FileData::Text { .. }) && !is_full_media_type(&file.media_type) {
            "text/plain"
        } else {
            resolved_media_type.as_deref().unwrap_or(&file.media_type)
        };
    push_file_block(
        &b64,
        media_type,
        file.filename.as_deref(),
        &file.provider_options,
        content,
        doc_counter,
    );
    if let FileData::Url { url, .. } = &file.data
        && let Some(block) = content.last_mut()
    {
        let inner = if block.get("guardContent").is_some() {
            &mut block["guardContent"]
        } else {
            block
        };
        let key = if media_type.starts_with("video/") {
            "video"
        } else {
            "image"
        };
        inner[key]["source"] = json!({"s3Location":{"uri":url}});
    }
}

/// Build a Bedrock `image` or `document` block from an already-base64 `bytes`
/// string. Images become `{ image: { format, source: { bytes } } }`; everything
/// else becomes a RAG `{ document: { format, name, source: { bytes } [, citations] } }`
/// block, mirroring the TS file-part handling (name = stripped filename or a
/// monotonically-incrementing `document-N`).
fn push_file_block(
    b64: &str,
    media_type: &str,
    filename: Option<&str>,
    provider_options: &Option<SharedProviderOptions>,
    content: &mut Vec<Value>,
    doc_counter: &mut u32,
) {
    let top_level = media_type.split('/').next().unwrap_or("");
    if top_level == "image" {
        let format = mime_to_image_format(media_type);
        let block = json!({ "image": { "format": format, "source": { "bytes": b64 } } });
        if options::read(provider_options.as_ref())
            .and_then(|v| v.get("guardContent"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            content.push(json!({"guardContent":block}));
        } else {
            content.push(block);
        }
    } else if top_level == "video" {
        content.push(
            json!({"video":{"format":mime_to_video_format(media_type),"source":{"bytes":b64}}}),
        );
    } else {
        let format = mime_to_document_format(media_type);
        let name = match filename {
            Some(f) if !sanitize_document_name(f).is_empty() => sanitize_document_name(f),
            _ => {
                *doc_counter += 1;
                format!("document-{}", *doc_counter)
            }
        };
        let mut doc = serde_json::Map::new();
        doc.insert("format".to_string(), json!(format));
        doc.insert("name".to_string(), json!(name));
        doc.insert("source".to_string(), json!({ "bytes": b64 }));
        if citations_enabled(provider_options) {
            doc.insert("citations".to_string(), json!({ "enabled": true }));
        }
        content.push(json!({ "document": Value::Object(doc) }));
    }
}

/// Extract a `{ cachePoint: {...} }` block from a part's `providerOptions`
/// (`amazonBedrock.cachePoint`), if present.
fn assistant_cache_point(part: &AssistantPart) -> Option<Value> {
    let po = match part {
        AssistantPart::Text(part) => &part.provider_options,
        AssistantPart::File(part) if !matches!(part.data, FileData::Reference { .. }) => {
            &part.provider_options
        }
        AssistantPart::Reasoning(part) => &part.provider_options,
        AssistantPart::ToolCall(part) => &part.provider_options,
        AssistantPart::ToolResult(part) => &part.provider_options,
        _ => return None,
    };
    cache_point(po.as_ref())
}

/// Whether `citations.enabled` is set on a part's `amazonBedrock` provider
/// options.
fn citations_enabled(provider_options: &Option<SharedProviderOptions>) -> bool {
    options::read(provider_options.as_ref())
        .and_then(|b| b.get("citations"))
        .and_then(|c| c.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Resolve a tool result `output` value into Bedrock's `toolResult.content`
/// array, mirroring the TS SDK.
fn resolve_tool_result_output(output: &ToolResultOutput, doc_counter: &mut u32) -> Vec<Value> {
    match output {
        ToolResultOutput::Json { value, .. } | ToolResultOutput::ErrorJson { value, .. } => {
            vec![json!({ "text": value.to_string() })]
        }
        ToolResultOutput::Text { value, .. } | ToolResultOutput::ErrorText { value, .. } => {
            vec![json!({ "text": value })]
        }
        ToolResultOutput::ExecutionDenied { reason, .. } => {
            vec![json!({ "text": reason.as_deref().unwrap_or("Tool call execution denied.") })]
        }
        ToolResultOutput::Content { value } => {
            let mut content = Vec::new();
            for part in value {
                match part {
                    ToolResultContent::Text(part) => content.push(json!({ "text": part.text })),
                    ToolResultContent::File(part) => {
                        let mut resolved = part.clone();
                        if let Ok(media_type) = aimux_provider_utils::resolve_full_media_type(part)
                        {
                            resolved.media_type = media_type;
                        }
                        let part = &resolved;
                        if let FileData::Url { url, .. } = &part.data {
                            let format = mime_to_video_format(&part.media_type).unwrap_or("");
                            let source = json!({ "s3Location": { "uri": url } });
                            content.push(if part.media_type.starts_with("image/") { json!({ "image": { "format": mime_to_image_format(&part.media_type), "source": source } }) } else { json!({ "video": { "format": format, "source": source } }) });
                        } else if part.media_type.starts_with("video/") {
                            if let FileData::Data { data } = &part.data {
                                let bytes = match data {
                                    FileBytes::Binary(bytes) => {
                                        base64::engine::general_purpose::STANDARD.encode(bytes)
                                    }
                                    FileBytes::Base64(data) => data.clone(),
                                };
                                content.push(json!({ "video": { "format": mime_to_video_format(&part.media_type).unwrap_or(""), "source": { "bytes": bytes } } }));
                            }
                        } else {
                            push_file_part(part, &mut content, doc_counter);
                        }
                    }
                    ToolResultContent::Custom { .. } => {}
                }
            }
            content
        }
    }
}

fn sanitize_tool_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn mime_to_image_format(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpeg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn mime_to_document_format(media_type: &str) -> &'static str {
    match media_type {
        "application/pdf" => "pdf",
        "text/csv" => "csv",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "text/html" => "html",
        "text/plain" => "txt",
        "text/markdown" => "md",
        _ => "txt",
    }
}

fn cache_point(provider_options: Option<&SharedProviderOptions>) -> Option<Value> {
    Some(json!({"cachePoint": options::read(provider_options)?.get("cachePoint")?}))
}

fn append_user_message(messages: &mut Vec<Value>, content: Vec<Value>) {
    if let Some(last) = messages.last_mut()
        && last["role"] == "user"
        && last["content"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|b| b.get("toolResult").is_some()))
    {
        last["content"].as_array_mut().unwrap().extend(content);
    } else {
        messages.push(json!({"role":"user","content":content}));
    }
}

fn flush_assistant(messages: &mut Vec<Value>, content: &mut Vec<Value>) {
    if content.iter().any(|v| v.get("cachePoint").is_none()) {
        messages.push(json!({"role":"assistant","content":std::mem::take(content)}));
    } else {
        content.clear();
    }
}

fn sanitize_document_name(name: &str) -> String {
    aimux_provider_utils::strip_file_extension(name)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " ()[]-".contains(*c))
        .collect::<String>()
        .trim()
        .chars()
        .take(200)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn mime_to_video_format(media_type: &str) -> Option<&'static str> {
    match media_type {
        "video/x-matroska" => Some("mkv"),
        "video/quicktime" => Some("mov"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/x-flv" => Some("flv"),
        "video/mpeg" => Some("mpeg"),
        "video/mpg" => Some("mpg"),
        "video/wmv" | "video/x-ms-wmv" => Some("wmv"),
        "video/3gpp" => Some("three_gp"),
        _ => None,
    }
}

fn validate_media_type(media_type: &str) -> Result<(), aimux_core::AiMuxError> {
    let supported = match media_type.split('/').next() {
        Some("image") => matches!(
            media_type,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ),
        Some("video") => mime_to_video_format(media_type).is_some(),
        _ => matches!(
            media_type,
            "application/pdf"
                | "text/csv"
                | "application/msword"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.ms-excel"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "text/html"
                | "text/plain"
                | "text/markdown"
        ),
    };
    if !supported {
        let kind = match media_type.split('/').next() {
            Some("image") => "image",
            Some("video") => "video",
            _ => "file",
        };
        return Err(aimux_core::AiMuxError::UnsupportedFunctionality(format!(
            "{kind} mime type: {media_type}"
        )));
    }
    Ok(())
}

/// Whether a media type is an image or a video, the only kinds Bedrock takes
/// from an S3 location.
fn is_image_or_video(media_type: &str) -> bool {
    media_type.starts_with("image/") || media_type.starts_with("video/")
}

/// Reject prompts the Converse API cannot express.
///
/// # Errors
///
/// An unsupported message arrangement.
pub fn validate_prompt(prompt: &LanguageModelPrompt) -> Result<(), aimux_core::AiMuxError> {
    let mut saw_non_system = false;
    for message in prompt {
        match message {
            LanguageModelMessage::System { .. } => {
                if saw_non_system {
                    return Err(aimux_core::AiMuxError::UnsupportedFunctionality(
                        "Multiple system messages that are separated by user/assistant messages"
                            .into(),
                    ));
                }
            }
            LanguageModelMessage::User { content, .. } => {
                saw_non_system = true;
                for part in content {
                    match part {
                        UserPart::Text(part) => {
                            validate_part_options(part.provider_options.as_ref(), false)?
                        }
                        UserPart::File(file) => validate_file(file)?,
                    }
                }
            }
            LanguageModelMessage::Assistant { content, .. } => {
                saw_non_system = true;
                for part in content {
                    match part {
                        AssistantPart::Text(part) => {
                            validate_part_options(part.provider_options.as_ref(), false)?
                        }
                        AssistantPart::File(file) => validate_file(file)?,
                        AssistantPart::Reasoning(part) => {
                            validate_part_options(part.provider_options.as_ref(), true)?
                        }
                        AssistantPart::ToolResult(part) => validate_tool_result(&part.output)?,
                        AssistantPart::ToolCall(_)
                        | AssistantPart::ReasoningFile(_)
                        | AssistantPart::Custom(_) => {}
                    }
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                saw_non_system = true;
                for part in content {
                    if let ToolPart::ToolResult(part) = part {
                        validate_tool_result(&part.output)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_part_options(
    provider_options: Option<&SharedProviderOptions>,
    reasoning: bool,
) -> Result<(), aimux_core::AiMuxError> {
    if let Some(options) = options::read(provider_options) {
        if let Some(value) = options.get("guardContent")
            && !value.is_boolean()
        {
            return Err(aimux_core::AiMuxError::invalid_argument(
                "Invalid Bedrock guardContent",
            ));
        }
        if let Some(value) = options.get("guardContentQualifiers")
            && !value.as_array().is_some_and(|a| {
                a.iter().all(|v| {
                    matches!(
                        v.as_str(),
                        Some("grounding_source" | "query" | "guard_content")
                    )
                })
            })
        {
            return Err(aimux_core::AiMuxError::invalid_argument(
                "Invalid Bedrock guardContentQualifiers",
            ));
        }
        if let Some(value) = options.get("citations")
            && (!value.is_object() || value.get("enabled").is_some_and(|v| !v.is_boolean()))
        {
            return Err(aimux_core::AiMuxError::invalid_argument(
                "Invalid Bedrock citations",
            ));
        }
        if reasoning {
            for key in ["signature", "redactedContent", "redactedData"] {
                if let Some(value) = options.get(key)
                    && !value.is_string()
                    && !value.is_null()
                {
                    return Err(aimux_core::AiMuxError::invalid_argument(format!(
                        "Invalid Bedrock {key}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn validate_file(file: &FilePart) -> Result<(), aimux_core::AiMuxError> {
    if !matches!(file.data, FileData::Reference { .. }) {
        validate_part_options(file.provider_options.as_ref(), false)?;
    }
    match &file.data {
        FileData::Reference { .. } => Err(aimux_core::AiMuxError::UnsupportedFunctionality(
            "file parts with provider references".into(),
        )),
        FileData::Url { url, .. } => {
            let unsupported =
                || aimux_core::AiMuxError::UnsupportedFunctionality("File URL data".into());
            if !url.starts_with("s3:") {
                return Err(unsupported());
            }
            let media_type = aimux_provider_utils::resolve_full_media_type(file)?;
            if !is_image_or_video(&media_type) {
                return Err(unsupported());
            }
            validate_media_type(&media_type)
        }
        FileData::Text { .. } => {
            let media_type = if is_full_media_type(&file.media_type) {
                &file.media_type
            } else {
                "text/plain"
            };
            if media_type.starts_with("image/") || media_type.starts_with("video/") {
                return Err(aimux_core::AiMuxError::UnsupportedFunctionality(format!(
                    "file media type or data: {media_type}"
                )));
            }
            validate_media_type(media_type)
        }
        FileData::Data { .. } => {
            let media_type = aimux_provider_utils::resolve_full_media_type(file)?;
            validate_media_type(&media_type)
        }
    }
}

fn validate_tool_result(result: &ToolResultOutput) -> Result<(), aimux_core::AiMuxError> {
    if let ToolResultOutput::Content { value } = result {
        for part in value {
            match part {
                ToolResultContent::Text(_) => {}
                ToolResultContent::File(file) => {
                    let kind = match &file.data {
                        FileData::Data { .. } => "data",
                        FileData::Url { .. } => "url",
                        FileData::Reference { .. } => "reference",
                        FileData::Text { .. } => "text",
                    };
                    let unsupported = || {
                        aimux_core::AiMuxError::UnsupportedFunctionality(format!(
                            "tool result file data of type \"{kind}\""
                        ))
                    };
                    let is_s3_url =
                        matches!(&file.data, FileData::Url { url, .. } if url.starts_with("s3:"));
                    if kind != "data" && !is_s3_url {
                        return Err(unsupported());
                    }
                    let media_type = aimux_provider_utils::resolve_full_media_type(file)?;
                    if is_s3_url && !is_image_or_video(&media_type) {
                        return Err(unsupported());
                    }
                    validate_media_type(&media_type)?;
                }
                ToolResultContent::Custom { .. } => {
                    return Err(aimux_core::AiMuxError::UnsupportedFunctionality(
                        "unsupported tool content part".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}
