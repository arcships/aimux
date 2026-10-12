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

use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, UserPart,
};
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified};
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
/// Returns an unsupported-functionality error for unsupported tool result content.
pub fn convert_prompt_to_bedrock(
    prompt: &LanguageModelPrompt,
) -> Result<(Vec<Value>, Vec<Value>), aimux_core::error::AiMuxError> {
    validate_tool_result_content(prompt)?;
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
                    if let Some(cp) = cache_point(provider_options) {
                        system.push(cp);
                    }
                }
            }
            Blk::User => {
                let mut content: Vec<Value> = Vec::new();
                for &i in idxs {
                    match &prompt[i] {
                        LanguageModelMessage::User { content: parts, .. } => {
                            for part in parts {
                                let provider_options = match part {
                                    UserPart::Text(part) => {
                                        content.push(json!({ "text": part.text }));
                                        &part.provider_options
                                    }
                                    UserPart::File(file) => {
                                        if !push_file_part(
                                            file,
                                            &mut content,
                                            &mut document_counter,
                                        ) {
                                            continue;
                                        }
                                        &file.provider_options
                                    }
                                };
                                if let Some(cp) = cache_point(provider_options) {
                                    content.push(cp);
                                }
                            }
                        }
                        LanguageModelMessage::Tool { content: parts, .. } => {
                            for part in parts {
                                let ToolPart::ToolResult(part) = part else {
                                    continue;
                                };
                                let result_content =
                                    resolve_tool_result_output(&part.output, &mut document_counter);
                                content.push(json!({
                                    "toolResult": {
                                        "toolUseId": part.tool_call_id,
                                        "content": result_content,
                                    }
                                }));
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                append_user_message(&mut messages, content);
            }
            Blk::Assistant => {
                let mut content: Vec<Value> = Vec::new();
                let num_msgs = idxs.len();
                for (mj, &i) in idxs.iter().enumerate() {
                    let is_last_message = mj == num_msgs - 1;
                    let LanguageModelMessage::Assistant { content: parts, .. } = &prompt[i] else {
                        unreachable!()
                    };
                    let has_reasoning = parts
                        .iter()
                        .any(|p| matches!(p, AssistantPart::Reasoning(_)));
                    let num_parts = parts.len();
                    for (kj, p) in parts.iter().enumerate() {
                        let is_last_content_part = kj == num_parts - 1;
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
                                let options = provider_options.as_ref().and_then(|options| {
                                    options
                                        .get("amazonBedrock")
                                        .or_else(|| options.get("bedrock"))
                                });
                                let Some(sig) = options
                                    .and_then(|options| options.get("signature"))
                                    .and_then(Value::as_str)
                                else {
                                    if let Some(redacted) =
                                        options.and_then(|options| options.get("redactedContent"))
                                    {
                                        content.push(json!({ "reasoningContent": { "redactedContent": redacted } }));
                                    } else if let Some(redacted) =
                                        options.and_then(|options| options.get("redactedData"))
                                    {
                                        content.push(json!({ "reasoningContent": { "redactedReasoning": { "data": redacted } } }));
                                    }
                                    continue;
                                };
                                // Only signed reasoning is replayed; unsigned
                                // reasoning is intentionally omitted.
                                content.push(json!({
                                    "reasoningContent": {
                                        "reasoningText": {
                                            "text": text,
                                            "signature": sig
                                        }
                                    }
                                }));
                            }
                            AssistantPart::ToolResult(part) => {
                                if !content.is_empty() {
                                    messages.push(json!({ "role": "assistant", "content": std::mem::take(&mut content) }));
                                }
                                let mut result = vec![
                                    json!({ "toolResult": { "toolUseId": part.tool_call_id, "content": resolve_tool_result_output(&part.output, &mut document_counter) } }),
                                ];
                                if let Some(cp) = cache_point(&part.provider_options) {
                                    result.push(cp);
                                }
                                append_user_message(&mut messages, result);
                                continue;
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
                                        "toolUseId": tool_call_id,
                                        "name": sanitize_tool_name(tool_name),
                                        "input": input_val,
                                    }
                                }));
                            }
                            _ => {}
                        }
                        let provider_options = match p {
                            AssistantPart::Text(part) => &part.provider_options,
                            AssistantPart::File(part)
                                if matches!(
                                    part.data,
                                    FileData::Data { .. } | FileData::Text { .. }
                                ) =>
                            {
                                &part.provider_options
                            }
                            _ => continue,
                        };
                        if let Some(cp) = cache_point(provider_options) {
                            content.push(cp);
                        }
                    }
                }
                if !content.is_empty() {
                    messages.push(json!({ "role": "assistant", "content": content }));
                }
            }
        }
    }

    Ok((system, messages))
}

fn append_user_message(messages: &mut Vec<Value>, content: Vec<Value>) {
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
        && let Some(parts) = last.get_mut("content").and_then(Value::as_array_mut)
    {
        parts.extend(content);
    } else {
        messages.push(json!({ "role": "user", "content": content }));
    }
}

pub(crate) fn validate_tool_result_content(
    prompt: &LanguageModelPrompt,
) -> Result<(), aimux_core::error::AiMuxError> {
    for message in prompt {
        let results: Vec<&ToolResultOutput> = match message {
            LanguageModelMessage::Tool { content, .. } => content
                .iter()
                .filter_map(|part| {
                    if let ToolPart::ToolResult(part) = part {
                        Some(&part.output)
                    } else {
                        None
                    }
                })
                .collect(),
            LanguageModelMessage::Assistant { content, .. } => content
                .iter()
                .filter_map(|part| {
                    if let AssistantPart::ToolResult(part) = part {
                        Some(&part.output)
                    } else {
                        None
                    }
                })
                .collect(),
            _ => continue,
        };
        for output in results {
            if let ToolResultOutput::Content { value } = output {
                for part in value {
                    let unsupported = match part {
                        ToolResultContent::Text(_) => false,
                        ToolResultContent::File(file) => {
                            let allowed_data = matches!(file.data, FileData::Data { .. })
                                || matches!(&file.data, FileData::Url { url, .. } if url.starts_with("s3:"));
                            if !allowed_data {
                                return Err(
                                    aimux_core::error::AiMuxError::UnsupportedFunctionality(
                                        "tool result content part".to_string(),
                                    ),
                                );
                            }
                            let media_type = crate::google::convert::tool_file_media_type(file)?;
                            let supported = matches!(
                                media_type.as_str(),
                                "image/jpeg"
                                    | "image/png"
                                    | "image/gif"
                                    | "image/webp"
                                    | "video/x-matroska"
                                    | "video/quicktime"
                                    | "video/mp4"
                                    | "video/webm"
                                    | "video/x-flv"
                                    | "video/mpeg"
                                    | "video/mpg"
                                    | "video/wmv"
                                    | "video/x-ms-wmv"
                                    | "video/3gpp"
                                    | "application/pdf"
                                    | "text/csv"
                                    | "application/msword"
                                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                                    | "application/vnd.ms-excel"
                                    | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                                    | "text/html"
                                    | "text/plain"
                                    | "text/markdown"
                            );
                            if !supported {
                                let kind = match media_type.split('/').next() {
                                    Some("image") => "image",
                                    Some("video") => "video",
                                    _ => "file",
                                };
                                return Err(
                                    aimux_core::error::AiMuxError::UnsupportedFunctionality(
                                        format!("{kind} mime type: {media_type}"),
                                    ),
                                );
                            }
                            matches!(file.data, FileData::Url { .. })
                                && !(media_type.starts_with("image/")
                                    || media_type.starts_with("video/"))
                        }
                        ToolResultContent::Custom { .. } => true,
                    };
                    if unsupported {
                        return Err(aimux_core::error::AiMuxError::UnsupportedFunctionality(
                            "tool result content part".to_string(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Push a supported file into a Bedrock content array.
fn push_file_part(file: &FilePart, content: &mut Vec<Value>, doc_counter: &mut u32) -> bool {
    let b64 = match &file.data {
        FileData::Data {
            data: FileBytes::Binary(bytes),
        } => base64::engine::general_purpose::STANDARD.encode(bytes),
        FileData::Data {
            data: FileBytes::Base64(data),
        } => data.clone(),
        FileData::Text { text } => base64::engine::general_purpose::STANDARD.encode(text),
        FileData::Url { url, .. }
            if url.starts_with("s3:") && file.media_type.starts_with("image/") =>
        {
            content.push(json!({
                "image": {
                    "format": mime_to_image_format(&file.media_type),
                    "source": { "s3Location": { "uri": url } }
                }
            }));
            return true;
        }
        FileData::Url { .. } | FileData::Reference { .. } => return false,
    };
    let is_text = matches!(file.data, FileData::Text { .. });
    let media_type = if is_text
        && !file
            .media_type
            .split_once('/')
            .is_some_and(|(_, subtype)| !subtype.is_empty() && subtype != "*")
    {
        "text/plain"
    } else {
        &file.media_type
    };
    push_file_block(
        &b64,
        media_type,
        file.filename.as_deref(),
        &file.provider_options,
        is_text,
        content,
        doc_counter,
    );
    true
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
    is_text: bool,
    content: &mut Vec<Value>,
    doc_counter: &mut u32,
) {
    let top_level = media_type.split('/').next().unwrap_or("");
    if top_level == "image" && !is_text {
        let format = mime_to_image_format(media_type);
        content.push(json!({
            "image": { "format": format, "source": { "bytes": b64 } }
        }));
    } else {
        let format = mime_to_document_format(media_type);
        let name = match filename {
            Some(f) => strip_file_extension(f),
            None => {
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
/// (`bedrock.cachePoint` or `amazonBedrock.cachePoint`), if present.
fn cache_point(provider_options: &Option<SharedProviderOptions>) -> Option<Value> {
    let po = provider_options.as_ref()?;
    for key in ["bedrock", "amazonBedrock"] {
        if let Some(cp) = po.get(key).and_then(|v| v.get("cachePoint")) {
            return Some(json!({ "cachePoint": cp.clone() }));
        }
    }
    None
}

/// Whether `citations.enabled` is set on a part's `bedrock`/`amazonBedrock`
/// provider options.
fn citations_enabled(provider_options: &Option<SharedProviderOptions>) -> bool {
    let Some(po) = provider_options.as_ref() else {
        return false;
    };
    for key in ["bedrock", "amazonBedrock"] {
        if let Some(enabled) = po
            .get(key)
            .and_then(|b| b.get("citations"))
            .and_then(|c| c.get("enabled"))
            .and_then(serde_json::Value::as_bool)
        {
            return enabled;
        }
    }
    false
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
                        if let Ok(media_type) = crate::google::convert::tool_file_media_type(part) {
                            resolved.media_type = media_type;
                        }
                        let part = &resolved;
                        if let FileData::Url { url, .. } = &part.data {
                            let format = part.media_type.split('/').nth(1).unwrap_or("");
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
                                content.push(json!({ "video": { "format": part.media_type.split('/').nth(1).unwrap_or(""), "source": { "bytes": bytes } } }));
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
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn strip_file_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
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

// ── Tools ───────────────────────────────────────────────────────────────────

/// Whether a Bedrock model id supports the `strict` tool schema field.
///
/// Mirrors the TS `supportsStrictTools`: the newest Claude models reject
/// `strict` (and `output_config.format`), so the field is omitted for them.
#[must_use]
pub fn supports_strict_tools(model_id: &str) -> bool {
    const REJECTING: &[&str] = &[
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-fable-5",
        "claude-sonnet-5",
    ];
    !REJECTING.iter().any(|m| model_id.contains(m))
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
/// Provider-defined tools (web_search, anthropic provider tools) and the
/// `additionalTools`/`betas`/`toolWarnings` they produce are not modelled in
/// the Rust `FunctionTool` and are intentionally not handled here.
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

/// Build the Bedrock Converse request body.
///
/// When `providerOptions.bedrock.reasoningConfig = { type: 'enabled',
/// budgetTokens: N }` is present, it is translated into
/// `additionalModelRequestFields.thinking = { type: 'enabled', budget_tokens: N }`
/// and `inferenceConfig.maxTokens` is bumped by `budgetTokens`. The
/// `reasoningConfig` key never appears in the request body. User-supplied
/// `additionalModelRequestFields` are merged with the derived `thinking` field.
///
/// # Errors
///
/// Returns an unsupported-functionality error for unsupported tool result content.
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
) -> Result<Value, aimux_core::error::AiMuxError> {
    let (system, messages) = convert_prompt_to_bedrock(&options.prompt)?;

    // Extract Bedrock-specific provider options.
    let bedrock_opts = options
        .provider_options
        .as_ref()
        .and_then(|po| po.get("amazonBedrock").or_else(|| po.get("bedrock")));
    let reasoning_config = bedrock_opts.and_then(|bo| bo.get("reasoningConfig"));
    let user_amrf = bedrock_opts.and_then(|bo| bo.get("additionalModelRequestFields"));

    // Derive the thinking config and budget_tokens from reasoningConfig.
    let mut thinking: Option<Value> = None;
    let mut budget_tokens: Option<u64> = None;
    if let Some(rc) = reasoning_config
        && rc.get("type").and_then(|v| v.as_str()) == Some("enabled")
        && let Some(bt) = rc.get("budgetTokens").and_then(serde_json::Value::as_u64)
    {
        thinking = Some(json!({ "type": "enabled", "budget_tokens": bt }));
        budget_tokens = Some(bt);
    }

    let mut inference_config = serde_json::Map::new();
    if let Some(max) = options.max_output_tokens {
        // When thinking is enabled, maxTokens is bumped by budgetTokens so the
        // model has room for both reasoning and the visible response.
        let max_tokens = budget_tokens.map(|bt| max + bt as u32).unwrap_or(max);
        inference_config.insert("maxTokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = options.temperature {
        inference_config.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        inference_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(top_k) = options.top_k {
        inference_config.insert("topK".to_string(), json!(top_k));
    }
    if let Some(ref stop) = options.stop_sequences {
        inference_config.insert("stopSequences".to_string(), json!(stop));
    }

    // Build additionalModelRequestFields: merge user-supplied fields with the
    // derived thinking config (thinking takes precedence on conflict).
    let mut amrf = serde_json::Map::new();
    if let Some(user_amrf) = user_amrf
        && let Some(obj) = user_amrf.as_object()
    {
        for (k, v) in obj {
            amrf.insert(k.clone(), v.clone());
        }
    }
    if let Some(thinking) = thinking {
        amrf.insert("thinking".to_string(), thinking);
    }

    let mut body = serde_json::Map::new();
    body.insert("messages".to_string(), Value::Array(messages));

    if !system.is_empty() {
        body.insert("system".to_string(), Value::Array(system));
    }
    if !inference_config.is_empty() {
        body.insert(
            "inferenceConfig".to_string(),
            Value::Object(inference_config),
        );
    }
    if !amrf.is_empty() {
        body.insert(
            "additionalModelRequestFields".to_string(),
            Value::Object(amrf),
        );
    }

    // Tools
    let function_tools: Option<Vec<FunctionTool>> = options.tools.as_ref().map(|tools| {
        tools
            .iter()
            .filter_map(|t| match t {
                Tool::Function(ft) => Some(ft.clone()),
                Tool::Provider(_) => None,
            })
            .collect()
    });
    let tool_config = prepare_tools(&function_tools, options.tool_choice.as_ref(), model_id);
    if tool_config
        .as_object()
        .map(|o| !o.is_empty())
        .unwrap_or(false)
    {
        body.insert("toolConfig".to_string(), tool_config);
    }

    Ok(Value::Object(body))
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
