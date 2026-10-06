//! Conversion between `LanguageModelPrompt` and Amazon Bedrock Converse format.
//!
//! Mirrors `convert-to-amazon-bedrock-chat-messages.ts`,
//! `amazon-bedrock-prepare-tools.ts`, `map-amazon-bedrock-finish-reason.ts`,
//! and `convert-amazon-bedrock-usage.ts` in the TS SDK. The Converse API uses a
//! unified message format across all model providers:
//!
//! - System messages are lifted out of `messages` into a top-level `system`
//!   array of `{ "text": "..." }` blocks.
//! - User and tool messages are merged into `role: "user"` messages.
//! - Assistant messages become `role: "assistant"`.
//! - Tool calls become `toolUse` blocks; tool results become `toolResult`
//!   blocks.
//! - Consecutive same-role messages are merged into a single message (matching
//!   the TS `groupIntoBlocks` behaviour).

use super::options;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified};
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
pub fn convert_prompt_to_bedrock(
    prompt: &LanguageModelPrompt,
) -> Result<(Vec<Value>, Vec<Value>), aimux_core::AiMuxError> {
    convert_prompt(prompt, false)
}

fn convert_prompt(
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

fn strip_file_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) => name[..i].to_string(),
        _ => name.to_string(),
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

#[must_use]
pub fn normalize_tool_call_id(id: &str, is_mistral: bool) -> String {
    if !is_mistral || (id.len() == 9 && id.bytes().all(|c| c.is_ascii_alphanumeric())) {
        return id.to_owned();
    }
    let mut hash = 14_695_981_039_346_656_037_u64;
    for unit in id.encode_utf16() {
        hash = (hash ^ u64::from(unit)).wrapping_mul(1_099_511_628_211);
    }
    let mut value = hash % 62_u64.pow(9);
    let mut bytes = [b'0'; 9];
    let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    for byte in bytes.iter_mut().rev() {
        *byte = alphabet[(value % 62) as usize];
        value /= 62;
    }
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn sanitize_document_name(name: &str) -> String {
    strip_file_extension(name)
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

fn is_strict_schema_compatible(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return true;
    };
    let is_object = schema["type"] == "object"
        || schema["type"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "object"));
    if is_object && schema["additionalProperties"] != false {
        return false;
    }
    for key in [
        "properties",
        "patternProperties",
        "definitions",
        "$defs",
        "dependencies",
    ] {
        if let Some(map) = object.get(key).and_then(Value::as_object)
            && map.values().any(|v| !is_strict_schema_compatible(v))
        {
            return false;
        }
    }
    for key in [
        "propertyNames",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "items",
        "anyOf",
        "allOf",
        "oneOf",
    ] {
        if let Some(value) = object.get(key) {
            if let Some(array) = value.as_array() {
                if array.iter().any(|v| !is_strict_schema_compatible(v)) {
                    return false;
                }
            } else if !is_strict_schema_compatible(value) {
                return false;
            }
        }
    }
    true
}

fn validate_media_type(
    media_type: &str,
    is_url: bool,
    url: Option<&str>,
) -> Result<(), aimux_core::AiMuxError> {
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
    if is_url
        && (!url.is_some_and(|u| u.starts_with("s3:"))
            || !(media_type.starts_with("image/") || media_type.starts_with("video/")))
    {
        return Err(aimux_core::AiMuxError::UnsupportedFunctionality(format!(
            "file media type or data: {media_type}"
        )));
    }
    Ok(())
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
            return Err(aimux_core::AiMuxError::InvalidArgument(
                "Invalid Bedrock guardContent".into(),
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
            return Err(aimux_core::AiMuxError::InvalidArgument(
                "Invalid Bedrock guardContentQualifiers".into(),
            ));
        }
        if let Some(value) = options.get("citations")
            && (!value.is_object() || value.get("enabled").is_some_and(|v| !v.is_boolean()))
        {
            return Err(aimux_core::AiMuxError::InvalidArgument(
                "Invalid Bedrock citations".into(),
            ));
        }
        if reasoning {
            for key in ["signature", "redactedContent", "redactedData"] {
                if let Some(value) = options.get(key)
                    && !value.is_string()
                    && !value.is_null()
                {
                    return Err(aimux_core::AiMuxError::InvalidArgument(format!(
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
            "File reference data".into(),
        )),
        FileData::Url { url, .. } => validate_media_type(&file.media_type, true, Some(url)),
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
            validate_media_type(media_type, false, None)
        }
        FileData::Data { .. } => {
            let media_type = aimux_provider_utils::resolve_full_media_type(file)?;
            validate_media_type(&media_type, false, None)
        }
    }
}

fn validate_tool_result(result: &ToolResultOutput) -> Result<(), aimux_core::AiMuxError> {
    if let ToolResultOutput::Content { value } = result {
        for part in value {
            match part {
                ToolResultContent::Text(_) => {}
                ToolResultContent::File(file) => {
                    let (is_url, url) = match &file.data {
                        FileData::Data { .. } => (false, None),
                        FileData::Url { url, .. } => (true, Some(url.as_str())),
                        _ => {
                            return Err(aimux_core::AiMuxError::UnsupportedFunctionality(
                                "tool result file data".into(),
                            ));
                        }
                    };
                    let media_type = aimux_provider_utils::resolve_full_media_type(file)?;
                    validate_media_type(&media_type, is_url, url)?;
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

// ── Tools ───────────────────────────────────────────────────────────────────

/// Whether a Bedrock model id supports the `strict` tool schema field.
///
/// Mirrors the TS `supportsStrictTools`: the newest Claude models reject
/// `strict` (and `output_config.format`), so the field is omitted for them.
#[must_use]
pub fn supports_strict_tools(model_id: &str) -> bool {
    !MODELS_WITHOUT_STRICT_TOOL_SUPPORT
        .iter()
        .any(|m| model_id.contains(m))
}

const MODELS_WITHOUT_STRICT_TOOL_SUPPORT: &[&str] = &[
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-fable-5",
    "claude-sonnet-5",
];

fn supports_native_structured_output(model_id: &str) -> bool {
    let prefix = MODELS_WITHOUT_STRICT_TOOL_SUPPORT[0]
        .split_once('-')
        .unwrap()
        .0;
    supports_strict_tools(model_id)
        && !["sonnet-4-6", "haiku-4-5"]
            .iter()
            .any(|suffix| model_id.contains(&format!("{prefix}-{suffix}")))
}

/// Prepare `FunctionTool`s into the Bedrock `toolConfig` JSON shape.
///
/// Mirrors the function-tool subset of the TS `prepareTools`:
/// - no tools (or `toolChoice: none`) → empty `toolConfig` `{}`
/// - `toolChoice: tool` filters tools to the named one
/// - `toolChoice: auto/required/tool` → `{ auto: {} }` / `{ any: {} }` /
///   `{ tool: { name } }`
/// - `description` is omitted when empty/whitespace
/// - `strict` is passed through only for models that `supports_strict_tools`
///
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&ToolChoice>,
    model_id: &str,
) -> Value {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    let Some(tools) = non_empty else {
        return json!({});
    };

    // `toolChoice: none` clears tools entirely (matches TS).
    if matches!(tool_choice, Some(ToolChoice::None)) {
        return json!({});
    }

    // `toolChoice: tool` filters function tools to the named one.
    let filtered: Vec<&FunctionTool> = match tool_choice {
        Some(ToolChoice::Tool { tool_name }) => {
            tools.iter().filter(|t| &t.name == tool_name).collect()
        }
        _ => tools.iter().collect(),
    };

    let supports_strict = supports_strict_tools(model_id);
    let tool_specs: Vec<Value> = filtered
        .iter()
        .map(|t| {
            let mut spec = serde_json::Map::new();
            spec.insert("name".to_string(), json!(t.name));
            if let Some(ref desc) = t.description
                && !desc.trim().is_empty()
            {
                spec.insert("description".to_string(), json!(desc));
            }
            if let Some(strict) = t.strict
                && supports_strict
                && (!strict || is_strict_schema_compatible(&t.input_schema))
            {
                spec.insert("strict".to_string(), json!(strict));
            }
            spec.insert("inputSchema".to_string(), json!({ "json": t.input_schema }));
            json!({ "toolSpec": Value::Object(spec) })
        })
        .collect();

    if tool_specs.is_empty() {
        return json!({});
    }

    let tool_choice_val = match tool_choice {
        Some(ToolChoice::Auto) => json!({ "auto": {} }),
        Some(ToolChoice::Required) => json!({ "any": {} }),
        Some(ToolChoice::Tool { tool_name }) => json!({ "tool": { "name": tool_name } }),
        Some(ToolChoice::None) => unreachable!(),
        None => return json!({ "tools": tool_specs }),
    };

    json!({ "tools": tool_specs, "toolChoice": tool_choice_val })
}

// ── Request body ────────────────────────────────────────────────────────────

// The request capabilities come from Anthropic's getModelCapabilities, not
// Bedrock's separate strict-tool support list. Unknown newer model IDs assume
// adaptive thinking and reject sampling; legacy IDs retain their defaults.
fn reasoning_capabilities(model_id: &str) -> (f64, bool, bool, bool) {
    let Some((_, model)) = model_id.split_once("claude-") else {
        return (4096.0, false, false, false);
    };
    if ["opus-5", "fable-5", "sonnet-5", "opus-4-8", "opus-4-7"]
        .iter()
        .any(|id| model.starts_with(id))
    {
        (128000.0, true, true, true)
    } else if ["sonnet-4-6", "opus-4-6"]
        .iter()
        .any(|id| model.starts_with(id))
    {
        (128000.0, true, false, true)
    } else if ["sonnet-4-5", "opus-4-5", "haiku-4-5"]
        .iter()
        .any(|id| model.starts_with(id))
    {
        (64000.0, false, false, true)
    } else if model.starts_with("opus-4-1") {
        (32000.0, false, false, true)
    } else if model.starts_with("sonnet-4-") || model.starts_with("sonnet-4@") {
        (64000.0, false, false, false)
    } else if model.starts_with("opus-4-") || model.starts_with("opus-4@") {
        (32000.0, false, false, false)
    } else {
        let two = model.strip_prefix('v').unwrap_or(model);
        let legacy = model == "instant"
            || model.starts_with("instant-")
            || two == "2"
            || two.starts_with("2-")
            || two.starts_with("2.")
            || two.starts_with("2:")
            || model == "3"
            || model.starts_with("3-")
            || model.starts_with("3.");
        if legacy {
            (4096.0, false, false, false)
        } else {
            (128000.0, true, true, true)
        }
    }
}

/// Build a checked Converse request, preserving upstream warnings.
///
/// # Errors
///
/// A prompt or tool definition the Converse API cannot express.
pub fn build_request_body_checked(
    model_id: &str,
    call: &CallOptions,
    model_family: Option<&str>,
) -> Result<(Value, Vec<aimux_core::types::Warning>, bool, bool), aimux_core::AiMuxError> {
    use aimux_core::types::{ReasoningEffort, Warning};
    let mut warnings = Vec::new();
    let mut compatibility = Vec::new();
    let mut warn = |feature: &str, details: Option<&str>| {
        warnings.push(Warning::Unsupported {
            feature: feature.into(),
            details: details.map(str::to_owned),
        })
    };
    let bedrock = options::read(call.provider_options.as_ref());
    validate_options(bedrock)?;
    let mut reasoning = bedrock
        .and_then(|b| b.get("reasoningConfig"))
        .cloned()
        .unwrap_or(json!({}));
    let anthropic = model_family == Some("anthropic")
        || model_id.contains("anthropic")
        || (model_id.contains(":application-inference-profile/")
            && reasoning.get("budgetTokens").is_some());
    let openai = model_id
        .split('.')
        .any(|p| p == options::OPENAI_MODEL_FAMILY);
    let oss = openai && model_id.contains("-oss-");
    let nova = model_id.contains("amazon.nova-2-lite-v1:0");
    let (max_reasoning_tokens, adaptive, rejects_sampling, supports_structured_output) =
        reasoning_capabilities(model_id);
    if let Some(effort) = &call.reasoning
        && effort.is_custom()
    {
        if anthropic && matches!(effort, ReasoningEffort::None) {
            reasoning = json!({"type":"disabled"});
        } else if !matches!(effort, ReasoningEffort::None)
            && (anthropic
                || openai
                || nova
                || bedrock.is_some_and(|b| b.contains_key("reasoningConfig")))
        {
            let level = match effort {
                ReasoningEffort::Minimal | ReasoningEffort::Low => "low",
                ReasoningEffort::Medium => "medium",
                ReasoningEffort::High => "high",
                ReasoningEffort::Xhigh => "max",
                _ => "low",
            };
            if (!anthropic || adaptive)
                && matches!(effort, ReasoningEffort::Minimal | ReasoningEffort::Xhigh)
            {
                compatibility.push(Warning::Compatibility { feature: "reasoning".into(), details: Some(format!("reasoning \"{effort}\" is not directly supported by this model. mapped to effort \"{level}\".")) });
            }
            let mut derived = if anthropic && adaptive {
                json!({"type":"adaptive","maxReasoningEffort":level})
            } else if anthropic {
                let percentage = match effort {
                    ReasoningEffort::Minimal => 0.02,
                    ReasoningEffort::Low => 0.1,
                    ReasoningEffort::Medium => 0.3,
                    ReasoningEffort::High => 0.6,
                    _ => 0.9,
                };
                json!({"type":"enabled","budgetTokens":(max_reasoning_tokens * percentage).round().clamp(1024.0, max_reasoning_tokens) as u64})
            } else {
                json!({"maxReasoningEffort":level})
            };
            if nova {
                derived["type"] = json!("enabled");
            }
            if let Some(explicit) = reasoning.as_object() {
                derived.as_object_mut().unwrap().extend(explicit.clone());
            }
            reasoning = derived;
        } else if !matches!(effort, ReasoningEffort::None) {
            warn(
                "reasoning",
                Some(
                    "Portable reasoning is not supported for this model and will be ignored. If the model supports a provider-specific reasoning configuration, use providerOptions.amazonBedrock.reasoningConfig.",
                ),
            );
        }
        if reasoning["type"] == "disabled" {
            reasoning.as_object_mut().unwrap().remove("budgetTokens");
            reasoning
                .as_object_mut()
                .unwrap()
                .remove("maxReasoningEffort");
        }
    }
    let thinking = anthropic && matches!(reasoning["type"].as_str(), Some("enabled" | "adaptive"));
    let rejects_forced = anthropic
        && (model_id.contains("sonnet-5-5")
            || model_id.contains("opus-5-5")
            || model_id.contains("fable-5-1"));
    let mut inference = serde_json::Map::new();
    if let Some(max) = call.max_output_tokens {
        inference.insert("maxTokens".into(), json!(max));
    }
    if let Some(temp) = call.temperature {
        if rejects_sampling || thinking || (openai && !oss) {
            let details = if rejects_sampling {
                format!("temperature is not supported by {model_id} and will be ignored")
            } else if thinking {
                "temperature is not supported when thinking is enabled".into()
            } else {
                "temperature is not supported by this OpenAI model on the Converse API".into()
            };
            warn("temperature", Some(&details));
        } else {
            let temp = if !openai || oss {
                if !(0.0..=1.0).contains(&temp) {
                    warn(
                        "temperature",
                        Some("temperature clamped to the Bedrock range [0, 1]"),
                    );
                }
                temp.clamp(0.0, 1.0)
            } else {
                temp
            };
            inference.insert("temperature".into(), json!(temp));
        }
    }
    if let Some(p) = call.top_p {
        if rejects_sampling || thinking || (openai && !oss) {
            let details = if rejects_sampling {
                format!("topP is not supported by {model_id} and will be ignored")
            } else if thinking {
                "topP is not supported when thinking is enabled".into()
            } else {
                "topP is not supported by this OpenAI model on the Converse API".into()
            };
            warn("topP", Some(&details));
        } else {
            inference.insert("topP".into(), json!(p));
        }
    }
    if let Some(k) = call.top_k {
        if rejects_sampling || thinking {
            let details = if rejects_sampling {
                format!("topK is not supported by {model_id} and will be ignored")
            } else {
                "topK is not supported when thinking is enabled".into()
            };
            warn("topK", Some(&details));
        } else {
            inference.insert("topK".into(), json!(k));
        }
    }
    if let Some(stop) = &call.stop_sequences {
        if openai {
            warn(
                "stopSequences",
                Some("stopSequences is not supported by this OpenAI model on the Converse API"),
            );
        } else {
            inference.insert("stopSequences".into(), json!(stop));
        }
    }
    for (feature, present) in [
        ("frequencyPenalty", call.frequency_penalty.is_some()),
        ("presencePenalty", call.presence_penalty.is_some()),
        ("seed", call.seed.is_some()),
    ] {
        if present {
            warn(feature, None);
        }
    }
    let mut fields = bedrock
        .and_then(|b| b.get("additionalModelRequestFields"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if thinking && reasoning["type"] == "enabled" {
        if let Some(budget) = reasoning["budgetTokens"].as_u64() {
            let max = inference
                .get("maxTokens")
                .and_then(Value::as_u64)
                .unwrap_or(4096);
            inference.insert("maxTokens".into(), json!(max.saturating_add(budget)));
            fields.insert(
                "thinking".into(),
                json!({"type":"enabled","budget_tokens":budget}),
            );
        }
    } else if thinking && reasoning["type"] == "adaptive" {
        let mut config = json!({"type":"adaptive"});
        if let Some(display) = reasoning.get("display") {
            config["display"] = display.clone();
        }
        fields.insert("thinking".into(), config);
    } else if !anthropic {
        if reasoning.get("budgetTokens").is_some() {
            warn(
                "budgetTokens",
                Some(
                    "budgetTokens applies only to Anthropic models on Bedrock and will be ignored for this model.",
                ),
            );
        }
        if reasoning["type"] == "adaptive" {
            warn(
                "adaptive thinking",
                Some("adaptive thinking type applies only to Anthropic models on Bedrock."),
            );
        }
    }
    if let Some(effort) = reasoning.get("maxReasoningEffort") {
        let key = if anthropic {
            "output_config"
        } else if openai && !oss {
            "reasoning"
        } else {
            "reasoningConfig"
        };
        if oss {
            fields.insert("reasoning_effort".into(), effort.clone());
        } else {
            let mut config = fields
                .get(key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            config.insert(
                if anthropic || openai {
                    "effort"
                } else {
                    "maxReasoningEffort"
                }
                .into(),
                effort.clone(),
            );
            if !anthropic && !openai {
                if let Some(kind) = reasoning.get("type").filter(|t| *t != "adaptive") {
                    config.insert("type".into(), kind.clone());
                }
                if let Some(budget) = reasoning
                    .get("budgetTokens")
                    .filter(|_| reasoning["type"] == "enabled")
                {
                    config.insert("budgetTokens".into(), budget.clone());
                }
            }
            fields.insert(key.into(), Value::Object(config));
        }
        if nova
            && reasoning["type"] == "enabled"
            && matches!(effort.as_str(), Some("high" | "xhigh" | "max"))
            && inference.remove("maxTokens").is_some()
        {
            warn(
                "maxOutputTokens",
                Some(
                    "maxOutputTokens is not supported when high reasoning is enabled and will be ignored",
                ),
            );
        }
    }
    if let Some(betas) = bedrock.and_then(|b| b.get("anthropicBeta")) {
        fields.insert("anthropic_beta".into(), betas.clone());
    }
    let anthropic_options = call
        .provider_options
        .as_ref()
        .and_then(|p| p.get("anthropic"));
    if let Some(value) = anthropic_options.and_then(|p| p.get("structuredOutputMode"))
        && !value
            .as_str()
            .is_some_and(|v| ["outputFormat", "jsonTool", "auto"].contains(&v))
    {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "Invalid Anthropic structuredOutputMode".into(),
        ));
    }
    if anthropic_options
        .and_then(|p| p.get("disableParallelToolUse"))
        .is_some_and(|v| !v.is_boolean())
    {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "Invalid Anthropic disableParallelToolUse".into(),
        ));
    }
    let structured_mode = bedrock
        .and_then(|b| b.get("structuredOutputMode"))
        .or_else(|| anthropic_options.and_then(|p| p.get("structuredOutputMode")))
        .and_then(Value::as_str)
        .unwrap_or("auto");
    if structured_mode == "jsonTool"
        && let Some(output_config) = fields
            .get_mut("output_config")
            .and_then(Value::as_object_mut)
    {
        output_config.remove("format");
        if output_config.is_empty() {
            fields.remove("output_config");
        }
    }
    let schema = match &call.response_format {
        Some(ResponseFormat::Json { schema, .. }) => schema.as_ref(),
        _ => None,
    };
    let supports_native = supports_native_structured_output(model_id);
    let uses_native = anthropic
        && schema.is_some()
        && (structured_mode == "outputFormat"
            || (structured_mode == "auto"
                && supports_native
                && (supports_structured_output || thinking || model_family == Some("anthropic"))));
    let uses_json_instruction = !uses_native
        && anthropic
        && schema.is_some()
        && (rejects_forced
            || (structured_mode != "jsonTool"
                && !supports_strict_tools(model_id)
                && call.tools.as_ref().is_some_and(|tools| !tools.is_empty())));
    let uses_json_tool = schema.is_some() && !uses_native && !uses_json_instruction;
    if uses_native {
        let output_config = fields.entry("output_config").or_insert_with(|| json!({}));
        if !output_config.is_object() {
            *output_config = json!({});
        }
        output_config["format"] = json!({"type":"json_schema", "schema":crate::anthropic::sanitize_json_schema::sanitize_json_schema(schema.unwrap())});
    }
    let mut all_tools = call.tools.clone().unwrap_or_default();
    if uses_json_tool {
        all_tools.push(Tool::Function(
            FunctionTool::new("json", schema.unwrap().clone())
                .with_description("Respond with a JSON object."),
        ));
    }
    let tool_choice = if uses_json_tool {
        Some(ToolChoice::Required)
    } else {
        call.tool_choice.clone()
    };
    let disable_parallel = anthropic_options
        .and_then(|p| p.get("disableParallelToolUse"))
        .and_then(Value::as_bool)
        == Some(true);
    let mut provider_tools = super::tools::prepare_provider_tools(
        Some(&all_tools),
        tool_choice.as_ref(),
        anthropic,
        disable_parallel,
        rejects_forced,
    )?;
    for warning in provider_tools.warnings.drain(..) {
        match warning {
            Warning::Unsupported { feature, details } => warn(&feature, details.as_deref()),
            other => compatibility.push(other),
        }
    }
    let tools: Option<Vec<FunctionTool>> = Some(
        all_tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(f) => Some(f.clone()),
                _ => None,
            })
            .collect(),
    );
    let function_choice =
        if provider_tools.using_anthropic_tools && matches!(tool_choice, Some(ToolChoice::None)) {
            Some(ToolChoice::Auto)
        } else {
            tool_choice.clone()
        };
    let mut tool_config = prepare_tools(&tools, function_choice.as_ref(), model_id);
    if !provider_tools.tools.is_empty() {
        let mut combined = provider_tools.tools;
        if let Some(functions) = tool_config.get("tools").and_then(Value::as_array) {
            combined.extend(functions.clone());
        }
        tool_config["tools"] = json!(combined);
    }
    let has_additional_tools = provider_tools.tool_choice.is_some();
    if provider_tools.using_anthropic_tools {
        tool_config.as_object_mut().unwrap().remove("toolChoice");
        if let Some(choice) = provider_tools.tool_choice {
            fields.insert("tool_choice".into(), choice);
        }
    }
    if !provider_tools.betas.is_empty() {
        let mut betas = bedrock
            .and_then(|b| b.get("anthropicBeta"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        betas.extend(provider_tools.betas.into_iter().map(Value::String));
        fields.insert("anthropic_beta".into(), json!(betas));
    }
    if !provider_tools.using_anthropic_tools
        && rejects_forced
        && matches!(tool_choice, Some(ToolChoice::Required | ToolChoice::Tool { .. }))
        && all_tools.iter().any(|tool| !matches!(tool, Tool::Provider(provider) if matches!(provider.id.as_str(), "anthropic.web_search_20250305" | "anthropic.web_search_20260318" | "anthropic.web_fetch_20260318")))
    {
        if tool_config.get("tools").is_some() { tool_config["toolChoice"] = json!({"auto":{}}); }
        let details = match &tool_choice {
            Some(ToolChoice::Tool {tool_name}) => format!("toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the '{tool_name}' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made."),
            _ => "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.".into(),
        };
        warn("toolChoice", Some(&details));
    }
    if let Some(tools) = &tools {
        for tool in tools.iter().filter(|tool| !matches!(&function_choice, Some(ToolChoice::Tool { tool_name }) if tool_name != &tool.name)) {
            if let Some(strict) = tool.strict
                && !supports_strict_tools(model_id)
            {
                warn(
                    "strict",
                    Some(&format!(
                        "Tool '{}' has strict: {}, but strict mode is not supported by this model on Amazon Bedrock. The strict property will be ignored.",
                        tool.name, strict
                    )),
                );
            } else if tool.strict == Some(true) && !is_strict_schema_compatible(&tool.input_schema)
            {
                warn(
                    "strict",
                    Some(&format!(
                        "Tool '{}' has strict: true, but Amazon Bedrock requires every object in a strict tool schema to set additionalProperties: false. The strict property will be ignored.",
                        tool.name
                    )),
                );
            }
        }
    }
    if !provider_tools.using_anthropic_tools
        && anthropic
        && disable_parallel
        && tool_config.get("tools").is_some()
    {
        let choice = if rejects_forced {
            json!({"type":"auto","disable_parallel_tool_use":true})
        } else {
            match &tool_choice {
                Some(ToolChoice::Required) => {
                    json!({"type":"any","disable_parallel_tool_use":true})
                }
                Some(ToolChoice::Tool { tool_name }) => {
                    json!({"type":"tool","name":tool_name,"disable_parallel_tool_use":true})
                }
                _ => json!({"type":"auto","disable_parallel_tool_use":true}),
            }
        };
        fields.insert("tool_choice".into(), choice);
        tool_config.as_object_mut().unwrap().remove("toolChoice");
    }
    let mut prompt = call.prompt.clone();
    if tool_config.get("tools").is_none() && !has_additional_tools {
        let has_tools = prompt.iter().any(|message| match message {
            LanguageModelMessage::Assistant { content, .. } => content.iter().any(|part| {
                matches!(
                    part,
                    AssistantPart::ToolCall(_) | AssistantPart::ToolResult(_)
                )
            }),
            LanguageModelMessage::Tool { content, .. } => !content.is_empty(),
            _ => false,
        });
        if has_tools {
            for message in &mut prompt {
                match message {
                    LanguageModelMessage::Assistant { content, .. } => content.retain(|part| {
                        !matches!(
                            part,
                            AssistantPart::ToolCall(_) | AssistantPart::ToolResult(_)
                        )
                    }),
                    LanguageModelMessage::Tool { content, .. } => content.clear(),
                    _ => {}
                }
            }
            prompt.retain(|message| match message {
                LanguageModelMessage::System { .. } => true,
                LanguageModelMessage::User { content, .. } => !content.is_empty(),
                LanguageModelMessage::Assistant { content, .. } => !content.is_empty(),
                LanguageModelMessage::Tool { content, .. } => !content.is_empty(),
            });
            warn(
                "toolContent",
                Some(
                    "Tool calls and results removed from conversation because Bedrock does not support tool content without active tools.",
                ),
            );
        }
    }
    if uses_json_instruction {
        let instruction = format!(
            "JSON schema:\n{}\nYou MUST answer with only a JSON object that matches the JSON schema above. Do not wrap it in markdown fences or include any other text.",
            schema.unwrap()
        );
        if let Some(LanguageModelMessage::System { content, .. }) = prompt.first_mut() {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&instruction);
        } else {
            prompt.insert(
                0,
                LanguageModelMessage::System {
                    content: instruction,
                    provider_options: None,
                },
            );
        }
    }
    let (system, messages) = convert_prompt(&prompt, model_id.contains("mistral."))?;
    let mut body = json!({"messages":messages});
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !inference.is_empty() {
        body["inferenceConfig"] = Value::Object(inference);
    }
    if !fields.is_empty() {
        body["additionalModelRequestFields"] = Value::Object(fields);
    }
    if anthropic {
        body["additionalModelResponseFieldPaths"] = json!(["/delta/stop_sequence"]);
    }
    if let Some(tier) = bedrock.and_then(|b| b.get("serviceTier")) {
        body["serviceTier"] = json!({"type":tier});
    }
    if let Some(bedrock) = bedrock {
        for (key, value) in bedrock {
            if !matches!(
                key.as_str(),
                "reasoningConfig"
                    | "additionalModelRequestFields"
                    | "serviceTier"
                    | "structuredOutputMode"
            ) {
                body[key] = value.clone();
            }
        }
    }
    if tool_config.get("tools").is_some() {
        body["toolConfig"] = tool_config;
    }
    warnings.extend(compatibility);
    Ok((body, warnings, uses_json_instruction, uses_json_tool))
}

fn validate_options(
    options: Option<&aimux_core::shared::JsonObject>,
) -> Result<(), aimux_core::AiMuxError> {
    let Some(options) = options else {
        return Ok(());
    };
    for (key, choices) in [
        (
            "structuredOutputMode",
            &["outputFormat", "jsonTool", "auto"][..],
        ),
        (
            "serviceTier",
            &["reserved", "priority", "default", "flex"][..],
        ),
    ] {
        if let Some(value) = options.get(key)
            && !value.as_str().is_some_and(|v| choices.contains(&v))
        {
            return Err(aimux_core::AiMuxError::InvalidArgument(format!(
                "Invalid Bedrock {key}"
            )));
        }
    }
    if let Some(value) = options.get("additionalModelRequestFields")
        && !value.is_object()
    {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "Invalid Bedrock additionalModelRequestFields".into(),
        ));
    }
    if let Some(value) = options.get("anthropicBeta")
        && !value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
    {
        return Err(aimux_core::AiMuxError::InvalidArgument(
            "Invalid Bedrock anthropicBeta".into(),
        ));
    }
    if let Some(reasoning) = options.get("reasoningConfig") {
        if !reasoning.is_object() {
            return Err(aimux_core::AiMuxError::InvalidArgument(
                "Invalid Bedrock reasoningConfig".into(),
            ));
        }
        for (key, choices) in [
            ("type", &["enabled", "disabled", "adaptive"][..]),
            ("display", &["omitted", "summarized"][..]),
            (
                "maxReasoningEffort",
                &["low", "medium", "high", "xhigh", "max"][..],
            ),
        ] {
            if let Some(value) = reasoning.get(key)
                && !value.as_str().is_some_and(|v| choices.contains(&v))
            {
                return Err(aimux_core::AiMuxError::InvalidArgument(format!(
                    "Invalid Bedrock reasoningConfig.{key}"
                )));
            }
        }
        if let Some(budget) = reasoning.get("budgetTokens")
            && !budget.is_number()
        {
            return Err(aimux_core::AiMuxError::InvalidArgument(
                "Invalid Bedrock reasoningConfig.budgetTokens".into(),
            ));
        }
    }
    Ok(())
}

/// Map a Bedrock `stopReason` to the unified `FinishReason`.
#[must_use]
pub fn map_finish_reason(reason: &str) -> FinishReason {
    let unified = match reason {
        "stop_sequence" | "end_turn" | "stop" => FinishReasonUnified::Stop,
        "max_tokens" | "length" => FinishReasonUnified::Length,
        "content_filtered" | "content-filter" | "guardrail_intervened" => {
            FinishReasonUnified::ContentFilter
        }
        "tool_use" | "tool-calls" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(reason.to_string()),
    }
}

/// Convert Bedrock usage to the core `Usage` type.
///
/// Mirrors the TS `convertAmazonBedrockUsage(usage: AmazonBedrockUsage |
/// undefined | null)`: a `None`/null/undefined usage yields an all-`None`
/// `Usage` (the TS `undefined` fields). The TS `raw` echo is not modelled on
/// the Rust `Usage` type (see `convert-usage` tests for the skipped `raw`
/// cases); `outputTokens.text` mirrors the TS `outputTokens.text` field.
#[must_use]
pub fn convert_usage(usage: Option<&BedrockUsage>) -> aimux_core::types::Usage {
    use aimux_core::types::Usage;

    let Some(usage) = usage else {
        return Usage::default();
    };

    let input = usage.input_tokens.unwrap_or(0);
    let output = usage.output_tokens.unwrap_or(0);
    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_write = usage.cache_write_input_tokens.unwrap_or(0);

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(input + cache_read + cache_write),
            no_cache: Some(input),
            cache_read: Some(cache_read),
            cache_write: Some(cache_write),
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(output),
            text: Some(output),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: serde_json::to_value(usage)
            .ok()
            .and_then(|value| value.as_object().cloned()),
    }
}

// Re-export for convenience.
pub use super::types::BedrockUsage;
