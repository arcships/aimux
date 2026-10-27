//! `convertToAnthropicPrompt` (`convert-to-anthropic-prompt.ts`): conversion
//! between `LanguageModelPrompt` and the Anthropic API format, with betas,
//! warnings and `sendReasoning` (mirroring the Vercel AI SDK).
//!
//! The conversion returns the Anthropic `system` + `messages` shape alongside
//! the beta headers and warnings produced during conversion. It supports
//! mid-conversation system messages, trailing-whitespace trimming on the final
//! assistant message, reasoning/thinking parts, URL & base64 file parts (with
//! PDF beta and top-level media-type sniffing), and provider-referenced files.

use indexmap::IndexSet;
use std::collections::{HashMap, HashSet};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, CustomPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart,
    TextPart, ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart,
    UserPart,
};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_core::types::Warning;
use aimux_provider_utils::{
    MediaTypeData, detect_media_type, get_top_level_media_type, is_full_media_type,
};
use serde_json::{Value, json};

use crate::anthropic::get_cache_control::CacheControlValidator;
use crate::anthropic::language_model_options::{CANONICAL, anthropic_options};
use aimux_provider_utils::ToolNameMapping;

/// Beta header emitted when a PDF file part is present.
const BETA_PDFS: &str = "pdfs-2024-09-25";
/// Beta header emitted when a system message appears mid-conversation.
const BETA_MID_CONVERSATION_SYSTEM: &str = "mid-conversation-system-2026-04-07";
/// Beta header emitted when a provider-referenced file part is present.
const BETA_FILES_API: &str = "files-api-2025-04-14";

/// Result of [`convert_prompt_to_anthropic_full_fallible`].
#[derive(Debug, Clone)]
pub struct AnthropicPromptConversion {
    /// Anthropic `system` blocks, or `None` when there is no system prompt.
    pub system: Option<Vec<Value>>,
    /// Anthropic `messages` array.
    pub messages: Vec<Value>,
    /// Beta headers required by the conversion (e.g. `pdfs-2024-09-25`).
    pub betas: IndexSet<String>,
    /// Warnings emitted while converting (e.g. unsupported reasoning metadata).
    pub warnings: Vec<Warning>,
}

/// Convert a prompt into the Anthropic `system` + `messages` shape, also
/// collecting the betas and warnings produced along the way.
///
/// Consecutive messages that map to the same effective Anthropic role
/// (`user`/`tool` → `user`, `assistant` → `assistant`) are merged into a single
/// message. A system message that appears *after* a non-system message is emitted
/// as an inline `{ "role": "system", ... }` message and adds the
/// `mid-conversation-system-2026-04-07` beta, matching the SDK behaviour.
///
/// `send_reasoning` controls whether assistant `Reasoning` parts are converted
/// into Anthropic `thinking` blocks (when `true`) or dropped with a warning
/// (when `false`).
///
/// # Errors
///
/// Returns `AiMuxError::invalid_argument` / `UnsupportedFunctionality` when a
/// message part cannot be represented in the Anthropic wire format (e.g. an
/// unresolvable file reference or an unsupported media type).
#[cfg(test)]
pub(super) fn convert_prompt_to_anthropic_full_fallible(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    convert_prompt_to_anthropic_full_with_tools(
        prompt,
        send_reasoning,
        &ToolNameMapping::default(),
        &HashMap::new(),
    )
}

/// [`convert_prompt_to_anthropic_full_fallible`] with the call's tool-name
/// mapping.
///
/// The mapping is only consulted for **assistant-role** `ToolResult` parts,
/// where the caller's tool name has to be resolved back to Anthropic's wire
/// name to pick the right provider-executed result block (`web_search_tool_result`,
/// `code_execution_tool_result`, …). Anthropic rejects a bare `tool_result`
/// block inside an assistant message with HTTP 400, so a result whose tool
/// cannot be resolved is dropped with a warning rather than emitted.
///
/// # Errors
///
/// Same as [`convert_prompt_to_anthropic_full_fallible`]: `AiMuxError::invalid_argument`
/// / `UnsupportedFunctionality` when a message part cannot be represented in the
/// Anthropic wire format.
#[cfg(test)]
pub(super) fn convert_prompt_to_anthropic_full_with_tools(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    toolset_names: &HashMap<String, String>,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    convert_prompt_for(prompt, send_reasoning, tool_names, toolset_names, CANONICAL)
}

/// [`convert_prompt_to_anthropic_full_with_tools`] for a provider whose
/// providerOptions key is `options_name`: the part and message options are
/// read from `anthropic` merged with `options_name` (the custom key wins) and
/// the metadata the response side wrote under `options_name` is read back.
///
/// # Errors
///
/// Same as [`convert_prompt_to_anthropic_full_with_tools`].
#[cfg(test)]
pub(super) fn convert_prompt_for(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    toolset_names: &HashMap<String, String>,
    options_name: &str,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    let mut validator = CacheControlValidator::for_options_name(options_name);
    convert_prompt_with_validator(
        prompt,
        send_reasoning,
        tool_names,
        toolset_names,
        options_name,
        &mut validator,
    )
}

pub(super) fn convert_prompt_with_validator(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    toolset_names: &HashMap<String, String>,
    options_name: &str,
    validator: &mut CacheControlValidator,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    // Ids of tool calls that were executed over MCP. Their results must be sent
    // back as `mcp_tool_result`, not as a provider-tool result block. Upstream
    // scopes this set to one merged assistant block; scanning the whole prompt
    // is a superset of that and is safe because tool call ids are unique.
    let mcp_tool_use_ids = collect_mcp_tool_use_ids(prompt, options_name);

    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut betas: IndexSet<String> = IndexSet::new();
    let mut warnings: Vec<Warning> = Vec::new();

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Eff {
        User,
        Assistant,
    }

    let mut seen_non_system = false;
    let mut last: Option<Eff> = None;
    let mut acc: Vec<Value> = Vec::new();

    fn flush(messages: &mut Vec<Value>, last: &mut Option<Eff>, acc: &mut Vec<Value>) {
        if let Some(role) = last.take()
            && !acc.is_empty()
        {
            let role_str = match role {
                Eff::User => "user",
                Eff::Assistant => "assistant",
            };
            let mut content = std::mem::take(acc);
            if role == Eff::Assistant {
                let mut ordered = Vec::new();
                let mut segment = Vec::new();
                let flush_segment = |segment: &mut Vec<Value>, ordered: &mut Vec<Value>| {
                    let (tool_uses, other): (Vec<_>, Vec<_>) =
                        std::mem::take(segment).into_iter().partition(|part| {
                            part.get("type").and_then(Value::as_str) == Some("tool_use")
                        });
                    ordered.extend(other);
                    ordered.extend(tool_uses);
                };
                for part in content {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("thinking" | "redacted_thinking" | "fallback")
                    ) {
                        flush_segment(&mut segment, &mut ordered);
                        ordered.push(part);
                    } else {
                        segment.push(part);
                    }
                }
                flush_segment(&mut segment, &mut ordered);
                content = ordered;
            }
            messages.push(json!({ "role": role_str, "content": content }));
        }
    }

    let mut has_system = false;
    // Index of the last Anthropic message holding a non-empty user message.
    let mut last_user_message_index: Option<usize> = None;
    for (index, msg) in prompt.iter().enumerate() {
        let eff = match msg {
            LanguageModelMessage::System { .. } => {
                if index > 0 && matches!(prompt[index - 1], LanguageModelMessage::System { .. }) {
                    continue;
                }
                flush(&mut messages, &mut last, &mut acc);
                let mut converted = Vec::new();
                for message in prompt[index..]
                    .iter()
                    .take_while(|message| matches!(message, LanguageModelMessage::System { .. }))
                {
                    let LanguageModelMessage::System {
                        content,
                        provider_options,
                    } = message
                    else {
                        unreachable!()
                    };
                    let options = anthropic_options(provider_options.as_ref(), options_name)
                        .unwrap_or_default();
                    let invalid =
                        || AiMuxError::invalid_argument("invalid anthropic provider options");
                    let clear_at = options.get("clearAt");
                    if clear_at.is_some_and(|value| value.as_str() != Some("next_user_message")) {
                        return Err(invalid());
                    }
                    let effort = options.get("effort");
                    if effort.is_some_and(|value| {
                        !matches!(
                            value.as_str(),
                            Some("low" | "medium" | "high" | "xhigh" | "max")
                        )
                    }) {
                        return Err(invalid());
                    }
                    let changes = match options.get("toolChanges") {
                        Some(value) => value.as_array().ok_or_else(invalid)?.as_slice(),
                        None => &[],
                    };
                    let mut blocks = Vec::new();
                    if !content.is_empty()
                        || (changes.is_empty() && clear_at.is_none() && effort.is_none())
                    {
                        let cc = validator.get_cache_control(
                            provider_options.as_ref(),
                            "system message",
                            true,
                        );
                        blocks.push(apply_cc(json!({ "type": "text", "text": content }), cc));
                    }
                    for change in changes {
                        let kind = change
                            .get("type")
                            .and_then(Value::as_str)
                            .filter(|kind| matches!(*kind, "tool_addition" | "tool_removal"))
                            .ok_or_else(invalid)?;
                        let name = change
                            .get("toolName")
                            .and_then(Value::as_str)
                            .ok_or_else(invalid)?;
                        blocks.push(json!({ "type": kind, "tool": {
                            "type": "tool_reference", "name": tool_names.to_provider_tool_name(name)
                        }}));
                    }
                    converted.push((blocks, clear_at.cloned(), effort.cloned(), changes.len()));
                }
                let change_count: usize = converted.iter().map(|message| message.3).sum();
                let has_controls = converted
                    .iter()
                    .any(|message| message.1.is_some() || message.2.is_some());
                if !seen_non_system || (!has_system && change_count == 0 && !has_controls) {
                    if change_count > 0 {
                        warnings.push(Warning::Other { message: "tool changes on the initial system message are not supported by Anthropic. Configure the initial tool set via the tools option instead. The tool changes have been ignored.".into() });
                    }
                    for (blocks, clear_at, effort, _) in converted {
                        if blocks.is_empty() && clear_at.is_none() && effort.is_some() {
                            messages.push(json!({ "role": "system", "content": [], "output_config": { "effort": effort } }));
                            betas.insert("mid-conversation-output-config-2026-07-01".into());
                        } else if clear_at.is_some() || effort.is_some() {
                            warnings.push(Warning::Other { message: "clearAt and effort on this initial system message are not supported by Anthropic. Use a separate effort-only system message with empty content to set effort. These options have been ignored.".into() });
                        }
                        system.extend(blocks.into_iter().filter(|block| block["type"] == "text"));
                    }
                    has_system = true;
                } else {
                    betas.insert(BETA_MID_CONVERSATION_SYSTEM.to_string());
                    for (blocks, clear_at, effort, changes) in converted {
                        let mut message = json!({ "role": "system", "content": blocks });
                        // Beta order of convert-to-anthropic-prompt.ts:273-282.
                        if changes > 0 {
                            betas.insert("mid-conversation-tool-changes-2026-07-01".into());
                        }
                        if let Some(clear_at) = clear_at {
                            message["clear_at"] = clear_at;
                            betas.insert("mid-conversation-system-clear-at-2026-08-21".into());
                        }
                        if let Some(effort) = effort {
                            message["output_config"] = json!({ "effort": effort });
                            betas.insert("mid-conversation-output-config-2026-07-01".into());
                        }
                        messages.push(message);
                    }
                }
                continue;
            }
            LanguageModelMessage::Assistant { .. } => Eff::Assistant,
            LanguageModelMessage::User { .. } | LanguageModelMessage::Tool { .. } => Eff::User,
        };
        seen_non_system = true;
        if last != Some(eff) {
            flush(&mut messages, &mut last, &mut acc);
        }
        match msg {
            LanguageModelMessage::System { .. } => unreachable!(),
            LanguageModelMessage::User {
                content,
                provider_options,
            } => {
                if !content.is_empty() {
                    last_user_message_index = Some(messages.len());
                }
                for (idx, part) in content.iter().enumerate() {
                    let is_last = idx + 1 == content.len();
                    let block = match part {
                        UserPart::Text(TextPart {
                            text,
                            provider_options: part_options,
                        }) => {
                            let cc = resolve_cache_control(
                                validator,
                                part_options.as_ref(),
                                provider_options.as_ref(),
                                is_last,
                                "user message part",
                                "user message",
                            );
                            apply_cc(json!({ "type": "text", "text": text }), cc)
                        }
                        UserPart::File(file) => convert_file_part(
                            file,
                            &mut betas,
                            validator,
                            provider_options.as_ref(),
                            is_last,
                            "user message part",
                            "user message",
                            options_name,
                        )?,
                    };
                    acc.push(block);
                }
            }
            LanguageModelMessage::Tool {
                content,
                provider_options,
            } => {
                for (idx, part) in content.iter().enumerate() {
                    let ToolPart::ToolResult(part) = part else {
                        continue;
                    };
                    acc.push(convert_tool_result(
                        part,
                        validator,
                        &mut betas,
                        &mut warnings,
                        idx + 1 == content.len(),
                        provider_options.as_ref(),
                        options_name,
                        toolset_names,
                    )?);
                }
            }
            LanguageModelMessage::Assistant {
                content,
                provider_options,
            } => {
                for (idx, part) in content.iter().enumerate() {
                    let is_last = idx + 1 == content.len();
                    if let AssistantPart::ToolResult(part) = part {
                        acc.extend(convert_assistant_tool_result(
                            part,
                            tool_names,
                            &mcp_tool_use_ids,
                            &mut warnings,
                            validator,
                            is_last,
                            provider_options.as_ref(),
                            options_name,
                        ));
                    } else if let Some(block) = convert_assistant_part(
                        part,
                        send_reasoning,
                        tool_names,
                        toolset_names,
                        &mut betas,
                        &mut warnings,
                        validator,
                        is_last,
                        provider_options.as_ref(),
                        options_name,
                    )? {
                        acc.push(block);
                    }
                }
            }
        }
        last = Some(eff);
    }
    flush(&mut messages, &mut last, &mut acc);

    // Anthropic does not allow trailing whitespace in pre-filled assistant
    // responses. When the final message is an assistant message, trim the last
    // text block (matching the TS SDK's `isLastBlock && isLastMessage &&
    // isLastContentPart` trim).
    if let Some(last_msg) = messages.last_mut()
        && last_msg.get("role").and_then(|r| r.as_str()) == Some("assistant")
        && let Some(content) = last_msg.get_mut("content").and_then(|c| c.as_array_mut())
        && let Some(last_block) = content.last_mut()
        && last_block.get("type").and_then(|t| t.as_str()) == Some("text")
        && let Some(text) = last_block.get("text").and_then(|t| t.as_str())
    {
        last_block["text"] = json!(text.trim());
    }

    normalize_orphaned_callers(&mut messages, last_user_message_index, &mut warnings);

    // Merge any cache_control validation warnings.
    warnings.extend(validator.take_warnings());

    let system_opt = has_system.then_some(system);
    Ok(AnthropicPromptConversion {
        system: system_opt,
        messages,
        betas,
        warnings,
    })
}

/// Drop the `caller` of historical assistant blocks whose source code
/// execution call is missing from the converted history (e.g. pruned), with
/// one warning per tool call. Only messages before the last user message are
/// normalized: tool-result messages do not end a turn, so an active code
/// execution keeps its callers (convert-to-anthropic-prompt.ts:1463-1510).
fn normalize_orphaned_callers(
    messages: &mut [Value],
    last_user_message_index: Option<usize>,
    warnings: &mut Vec<Warning>,
) {
    let Some(end) = last_user_message_index else {
        return;
    };
    let code_execution_ids: HashSet<String> = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|block| {
            block.get("type").and_then(Value::as_str) == Some("server_tool_use")
                && block.get("name").and_then(Value::as_str) == Some("code_execution")
        })
        .filter_map(|block| block.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut warned: HashSet<String> = HashSet::new();
    for message in messages.iter_mut().take(end) {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for block in blocks {
            let Some(caller) = block.get("caller") else {
                continue;
            };
            if caller.get("type").and_then(Value::as_str) == Some("direct") {
                continue;
            }
            let source = caller
                .get("tool_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if code_execution_ids.contains(source) {
                continue;
            }
            let source = source.to_string();
            let tool_call_id = block
                .get("id")
                .or_else(|| block.get("tool_use_id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(object) = block.as_object_mut() {
                object.remove("caller");
            }
            if warned.insert(tool_call_id.clone()) {
                warnings.push(Warning::Other {
                    message: format!(
                        "Omitted caller metadata for tool {tool_call_id} because source code execution tool {source} is missing from the conversation history."
                    ),
                });
            }
        }
    }
}

fn resolve_cache_control(
    validator: &mut CacheControlValidator,
    part_options: Option<&SharedProviderOptions>,
    message_options: Option<&SharedProviderOptions>,
    is_last_part: bool,
    part_context: &str,
    message_context: &str,
) -> Option<Value> {
    validator
        .get_cache_control(part_options, part_context, true)
        .or_else(|| {
            if is_last_part {
                validator.get_cache_control(message_options, message_context, true)
            } else {
                None
            }
        })
}

fn apply_cc(mut block: Value, cc: Option<Value>) -> Value {
    if let Some(cc) = cc {
        block["cache_control"] = cc;
    }
    block
}

#[allow(clippy::too_many_arguments)]
fn convert_file_part(
    file: &FilePart,
    betas: &mut IndexSet<String>,
    validator: &mut CacheControlValidator,
    message_options: Option<&SharedProviderOptions>,
    is_last_part: bool,
    part_context: &str,
    message_context: &str,
    options_name: &str,
) -> Result<Value, AiMuxError> {
    let FilePart {
        data,
        media_type,
        filename,
        provider_options,
    } = file;
    let resolve_inline_media_type = |data| -> Result<String, AiMuxError> {
        if is_full_media_type(media_type) {
            return Ok(media_type.clone());
        }
        if let Some(detected) = detect_media_type(data, Some(get_top_level_media_type(media_type)))?
        {
            return Ok(detected.into());
        }
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "file of media type \"{media_type}\" must specify subtype since it could not be auto-detected"
        )))
    };
    let mut block = match data {
        FileData::Data {
            data: FileBytes::Binary(bytes),
        } => {
            let full = if matches!(
                get_top_level_media_type(media_type),
                "image" | "application"
            ) {
                resolve_inline_media_type(MediaTypeData::Bytes(bytes))?
            } else {
                media_type.clone()
            };
            route_file_bytes(&full, bytes, filename.as_deref(), betas)?
        }
        FileData::Data {
            data: FileBytes::Base64(data),
        } => {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap_or_default();
            let full = if matches!(
                get_top_level_media_type(media_type),
                "image" | "application"
            ) {
                resolve_inline_media_type(MediaTypeData::Base64(data))?
            } else {
                media_type.clone()
            };
            route_file_base64(&full, data, &bytes, filename.as_deref(), betas)?
        }
        FileData::Url { url, .. } => route_file_url(media_type, url, betas)?,
        FileData::Reference { reference } => {
            let file_id = resolve_anthropic_reference(reference)?;
            betas.insert(BETA_FILES_API.to_string());
            let container_upload = anthropic_options(provider_options.as_ref(), options_name)
                .as_ref()
                .and_then(|a| a.get("containerUpload"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if container_upload {
                return Ok(json!({ "type": "container_upload", "file_id": file_id }));
            } else if get_top_level_media_type(media_type) == "image" {
                json!({ "type": "image", "source": { "type": "file", "file_id": file_id } })
            } else {
                json!({ "type": "document", "source": { "type": "file", "file_id": file_id } })
            }
        }
        FileData::Text { text } => {
            let document_options = anthropic_options(provider_options.as_ref(), options_name);
            let options = document_options.as_ref();
            let mut block = json!({ "type": "document", "source": {
                "type": "text", "media_type": "text/plain", "data": text,
            }});
            if let Some(title) = options
                .and_then(|o| o.get("title"))
                .and_then(Value::as_str)
                .or(filename.as_deref())
            {
                block["title"] = json!(title);
            }
            if let Some(context) = options
                .and_then(|o| o.get("context"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                block["context"] = json!(context);
            }
            if options
                .and_then(|o| o.get("citations"))
                .and_then(|c| c.get("enabled"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                block["citations"] = json!({ "enabled": true });
            }
            block
        }
    };
    if block.get("type").and_then(Value::as_str) == Some("document")
        && !matches!(data, FileData::Reference { .. })
        && let Some(metadata) = anthropic_options(provider_options.as_ref(), options_name)
    {
        for field in ["title", "context"] {
            if let Some(value) = metadata.get(field).filter(|value| {
                value.is_string() && (field == "title" || value.as_str() != Some(""))
            }) {
                block[field] = value.clone();
            }
        }
        if metadata
            .get("citations")
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            block["citations"] = json!({ "enabled": true });
        }
    }
    let cc = resolve_cache_control(
        validator,
        provider_options.as_ref(),
        message_options,
        is_last_part,
        part_context,
        message_context,
    );
    Ok(apply_cc(block, cc))
}

#[allow(clippy::too_many_arguments)]
fn convert_tool_result(
    part: &ToolResultPart,
    validator: &mut CacheControlValidator,
    betas: &mut IndexSet<String>,
    warnings: &mut Vec<Warning>,
    is_last_part: bool,
    message_provider_options: Option<&SharedProviderOptions>,
    options_name: &str,
    toolset_names: &HashMap<String, String>,
) -> Result<Value, AiMuxError> {
    let ToolResultPart {
        tool_call_id,
        tool_name,
        output,
        provider_options,
        ..
    } = part;

    let (content, is_error) = resolve_tool_result_output(output, betas, warnings, options_name)?;
    // Key order as upstream: `toolset_name` sits between `tool_use_id` and
    // `content` (convert-to-anthropic-prompt.ts:669-679).
    let mut block = json!({
        "type": "tool_result",
        "tool_use_id": tool_call_id,
    });
    if let Some(name) = toolset_name(toolset_names, tool_name, provider_options.as_ref()) {
        block["toolset_name"] = json!(name);
    }
    block["content"] = content;
    if is_error {
        block["is_error"] = json!(true);
    }
    // cache_control: part ?? output ?? (is_last_part ? message).
    let cc = validator
        .get_cache_control(provider_options.as_ref(), "tool result part", true)
        .or_else(|| {
            validator.get_cache_control(
                extract_tool_result_output_provider_options(output),
                "tool result output",
                true,
            )
        })
        .or_else(|| {
            is_last_part
                .then(|| {
                    validator.get_cache_control(
                        message_provider_options,
                        "tool result message",
                        true,
                    )
                })
                .flatten()
        });
    Ok(apply_cc(block, cc))
}

#[allow(clippy::too_many_arguments)]
fn convert_assistant_part(
    part: &AssistantPart,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    toolset_names: &HashMap<String, String>,
    betas: &mut IndexSet<String>,
    warnings: &mut Vec<Warning>,
    validator: &mut CacheControlValidator,
    is_last_part: bool,
    message_provider_options: Option<&SharedProviderOptions>,
    options_name: &str,
) -> Result<Option<Value>, AiMuxError> {
    let resolve_cc = |validator: &mut CacheControlValidator, part_opts| {
        resolve_cache_control(
            validator,
            part_opts,
            message_provider_options,
            is_last_part,
            "assistant message part",
            "assistant message",
        )
    };
    Ok(Some(match part {
        AssistantPart::Text(TextPart {
            text,
            provider_options,
        }) => {
            let cc = resolve_cc(validator, provider_options.as_ref());
            let metadata = anthropic_options(provider_options.as_ref(), options_name);
            let mut block = json!({ "type": "text", "text": text });
            if let Some(metadata) = metadata {
                if metadata.get("type").and_then(Value::as_str) == Some("compaction") {
                    if text.is_empty() {
                        return Ok(None);
                    }
                    block = json!({ "type": "compaction", "content": text });
                    if let Some(signature) =
                        metadata.get("signature").filter(|value| value.is_string())
                    {
                        block["signature"] = signature.clone();
                        betas.insert("compact-2026-09-04".to_string());
                    }
                } else if let Some(citations) = metadata.get("citations").filter(|v| !v.is_null()) {
                    block["citations"] = citations.clone();
                }
            }
            apply_cc(block, cc)
        }

        AssistantPart::File(file) => convert_file_part(
            file,
            betas,
            validator,
            message_provider_options,
            is_last_part,
            "assistant message part",
            "assistant message",
            options_name,
        )?,
        AssistantPart::Reasoning(ReasoningPart {
            text,
            provider_options,
        }) => {
            return Ok(convert_reasoning_part(
                text,
                provider_options.as_ref(),
                send_reasoning,
                warnings,
                validator,
                options_name,
            ));
        }

        AssistantPart::ToolCall(ToolCallPart {
            tool_call_id,
            tool_name,
            input,
            provider_executed,
            provider_options,
            ..
        }) => {
            let cc = resolve_cc(validator, provider_options.as_ref());

            if *provider_executed == Some(true) {
                let provider_name = tool_names.to_provider_tool_name(tool_name);
                let own_options = anthropic_options(provider_options.as_ref(), options_name);

                if own_options
                    .as_ref()
                    .and_then(|options| options.get("type"))
                    .and_then(Value::as_str)
                    == Some("mcp-tool-use")
                {
                    let Some(server_name) = own_options
                        .as_ref()
                        .and_then(|options| options.get("serverName"))
                        .and_then(Value::as_str)
                    else {
                        warnings.push(Warning::Other {
                            message: "mcp tool use server name is required and must be a string"
                                .to_string(),
                        });
                        return Ok(None);
                    };
                    return Ok(Some(apply_cc(
                        json!({
                            "type": "mcp_tool_use",
                            "id": tool_call_id,
                            "name": tool_name,
                            "input": input,
                            "server_name": server_name,
                        }),
                        cc,
                    )));
                }

                let (server_name, server_input) = if provider_name == "code_execution" {
                    let input_type = input.get("type").and_then(Value::as_str);
                    match input_type {
                        Some("bash_code_execution" | "text_editor_code_execution") => {
                            let mut value = input.clone();
                            if let Some(object) = value.as_object_mut() {
                                object.remove("type");
                            }
                            (input_type.unwrap().to_string(), value)
                        }
                        Some("programmatic-tool-call") => {
                            let mut value = input.clone();
                            if let Some(object) = value.as_object_mut() {
                                object.remove("type");
                            }
                            ("code_execution".to_string(), value)
                        }
                        _ => ("code_execution".to_string(), input.clone()),
                    }
                } else if matches!(
                    provider_name,
                    "web_fetch" | "web_search" | "tool_search_tool_regex" | "tool_search_tool_bm25"
                ) {
                    (provider_name.to_string(), input.clone())
                } else if provider_name == "advisor" {
                    ("advisor".to_string(), json!({}))
                } else {
                    warnings.push(Warning::Other {
                        message: format!(
                            "provider executed tool call for tool {tool_name} is not supported"
                        ),
                    });
                    return Ok(None);
                };

                let mut block = json!({
                    "type": "server_tool_use", "id": tool_call_id,
                    "name": server_name, "input": server_input,
                });
                if let Some(caller) = tool_caller(provider_options.as_ref(), options_name) {
                    block["caller"] = caller;
                }
                return Ok(Some(apply_cc(block, cc)));
            }

            // Anthropic requires `input` to be a JSON object. The SDK wraps any
            // non-object (e.g. malformed JSON the model produced) in
            // `{ "rawInvalidInput": <input> }`.
            let input_val = if input.is_object() {
                input.clone()
            } else {
                json!({ "rawInvalidInput": input })
            };
            let caller = tool_caller(provider_options.as_ref(), options_name);
            if let Some(toolset_name) =
                toolset_name(toolset_names, tool_name, provider_options.as_ref())
            {
                // A toolset member call: the `action` is the member tool
                // name. The input goes back as it is, `action` included, and
                // a call without one is still sent, named after the toolset
                // (convert-to-anthropic-prompt.ts:984-1004).
                let name = match input_val.get("action").and_then(Value::as_str) {
                    Some(action) => action.to_string(),
                    None => {
                        warnings.push(Warning::Other {
                            message: format!(
                                "toolset tool call for tool {tool_name} is missing the action"
                            ),
                        });
                        toolset_name.clone()
                    }
                };
                let mut block = json!({
                    "type": "tool_use",
                    "id": tool_call_id,
                    "name": name,
                    "toolset_name": toolset_name,
                    "input": input_val,
                });
                if let Some(caller) = caller {
                    block["caller"] = caller;
                }
                return Ok(Some(apply_cc(block, cc)));
            }
            let mut block = json!({
                "type": "tool_use",
                "id": tool_call_id,
                "name": tool_name,
                "input": input_val,
            });
            if let Some(caller) = caller {
                block["caller"] = caller;
            }
            apply_cc(block, cc)
        }

        // A server-side fallback boundary from a previous response; only the
        // canonical `anthropic` key is read, where the response side writes
        // it (convert-to-anthropic-prompt.ts:829-853).
        AssistantPart::Custom(CustomPart {
            kind,
            provider_options,
        }) if kind.as_str() == "anthropic.fallback" => {
            let metadata = provider_options
                .as_ref()
                .and_then(|options| options.get(CANONICAL));
            let model = |key: &str| {
                metadata?
                    .get(key)?
                    .get("model")?
                    .as_str()
                    .map(|model| json!({ "model": model }))
            };
            match (
                metadata.and_then(|metadata| metadata.get("type")),
                model("from"),
                model("to"),
            ) {
                (Some(Value::String(kind)), Some(from), Some(to)) if kind == "fallback" => {
                    json!({ "type": "fallback", "from": from, "to": to })
                }
                _ => {
                    warnings.push(Warning::Other {
                        message: "anthropic fallback metadata must include from.model and to.model"
                            .to_string(),
                    });
                    return Ok(None);
                }
            }
        }

        AssistantPart::ReasoningFile(_) | AssistantPart::Custom(_) => return Ok(None),
        AssistantPart::ToolResult(_) => unreachable!(),
    }))
}

// ── assistant-role tool results (provider-executed) ─────────────────────────

/// Collect the tool call ids that were executed over MCP.
///
/// Their results have to go back as `mcp_tool_result`, so they are indexed
/// before any message is converted. The marker is the one the response side
/// writes on an `mcp_tool_use` block (`language_model.rs`):
/// `providerOptions.anthropic.type == "mcp-tool-use"`.
fn collect_mcp_tool_use_ids<'a>(
    prompt: &'a LanguageModelPrompt,
    options_name: &str,
) -> HashSet<&'a str> {
    let mut ids = HashSet::new();
    for msg in prompt {
        let LanguageModelMessage::Assistant { content, .. } = msg else {
            continue;
        };
        for part in content {
            if let AssistantPart::ToolCall(ToolCallPart {
                tool_call_id,
                provider_options: Some(opts),
                ..
            }) = part
                && anthropic_options(Some(opts), options_name)
                    .as_ref()
                    .and_then(|a| a.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("mcp-tool-use")
            {
                ids.insert(tool_call_id.as_str());
            }
        }
    }
    ids
}

/// The Anthropic `toolset_name` of a tool call or tool result: from the tools
/// of this call, else from the `toolsetName` a previous response wrote. Only
/// the canonical `anthropic` key is read, where the response side writes it
/// (`getAnthropicToolsetName`, convert-to-anthropic-prompt.ts:1624-1641).
fn toolset_name(
    toolset_names: &HashMap<String, String>,
    tool_name: &str,
    provider_options: Option<&SharedProviderOptions>,
) -> Option<String> {
    toolset_names.get(tool_name).cloned().or_else(|| {
        provider_options?
            .get(CANONICAL)?
            .get("toolsetName")?
            .as_str()
            .map(str::to_string)
    })
}

fn tool_caller(
    provider_options: Option<&SharedProviderOptions>,
    options_name: &str,
) -> Option<Value> {
    anthropic_options(provider_options, options_name)
        .as_ref()
        .and_then(|anthropic| anthropic.get("caller"))
        .and_then(|caller| {
            let caller_type = caller.get("type")?.as_str()?;
            match caller_type {
                "code_execution_20250825" | "code_execution_20260120" => {
                    let tool_id = caller.get("toolId")?.as_str()?;
                    Some(json!({ "type": caller_type, "tool_id": tool_id }))
                }
                "direct" => Some(json!({ "type": "direct" })),
                _ => None,
            }
        })
}

/// Clone `key` out of `value`, defaulting to `null`.
pub(super) fn field(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

/// The error code carried by a result payload.
///
/// The response side emits `errorCode`; wire-shaped payloads that passed
/// through unmapped keep `error_code`. Both are accepted so a replayed result
/// round-trips either way.
fn result_error_code(value: &Value, fallback: &str) -> String {
    let parsed = value
        .as_str()
        .and_then(|value| serde_json::from_str::<Value>(value).ok());
    let value = parsed.as_ref().unwrap_or(value);
    value
        .get("errorCode")
        .or_else(|| value.get("error_code"))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

/// Convert an assistant-role `AssistantPart::ToolResult` into the matching
/// Anthropic provider-executed result block.
///
/// Anthropic only accepts a bare `tool_result` block inside a **user** message;
/// emitting one on an assistant message is a hard HTTP 400. This mirrors the TS
/// `convertToAnthropicPrompt` assistant `tool-result` branch (:871-1285): the
/// tool name is resolved back to Anthropic's wire name and dispatched to the
/// typed result block, and anything unrecognised is dropped with a warning
/// rather than sent as a bare `tool_result`.
///
/// Returns no block when the part is skipped, and two for an MCP result under
/// a provider tool name.
#[allow(clippy::too_many_arguments)]
fn convert_assistant_tool_result(
    part: &ToolResultPart,
    tool_names: &ToolNameMapping,
    mcp_tool_use_ids: &HashSet<&str>,
    warnings: &mut Vec<Warning>,
    validator: &mut CacheControlValidator,
    is_last_part: bool,
    message_provider_options: Option<&SharedProviderOptions>,
    options_name: &str,
) -> Vec<Value> {
    let ToolResultPart {
        tool_call_id,
        tool_name,
        output,
        provider_options,
        ..
    } = part;

    let cache_control = resolve_cache_control(
        validator,
        provider_options.as_ref(),
        message_provider_options,
        is_last_part,
        "assistant message part",
        "assistant message",
    );

    let raw_value = match output {
        ToolResultOutput::Json { value, .. } | ToolResultOutput::ErrorJson { value, .. } => {
            value.clone()
        }
        ToolResultOutput::Text { value, .. } | ToolResultOutput::ErrorText { value, .. } => {
            json!(value)
        }
        ToolResultOutput::ExecutionDenied { reason, .. } => {
            json!(reason.as_deref().unwrap_or("Tool call execution denied."))
        }
        ToolResultOutput::Content { value } => json!(value),
    };
    let provider_tool_name = tool_names.to_provider_tool_name(tool_name);
    let allowed = match output {
        ToolResultOutput::Json { .. } => true,
        ToolResultOutput::ErrorJson { .. } => {
            mcp_tool_use_ids.contains(tool_call_id.as_str())
                || matches!(
                    provider_tool_name,
                    "code_execution" | "web_fetch" | "web_search" | "advisor"
                )
        }
        ToolResultOutput::ErrorText { .. } => {
            provider_tool_name == "code_execution"
                && !mcp_tool_use_ids.contains(tool_call_id.as_str())
        }
        _ => false,
    };
    if !allowed {
        let known = mcp_tool_use_ids.contains(tool_call_id.as_str())
            || matches!(
                provider_tool_name,
                "code_execution"
                    | "web_fetch"
                    | "web_search"
                    | "tool_search_tool_regex"
                    | "tool_search_tool_bm25"
                    | "advisor"
            );
        let message = if known {
            let output_type = match output {
                ToolResultOutput::Text { .. } => "text",
                ToolResultOutput::Json { .. } => "json",
                ToolResultOutput::ExecutionDenied { .. } => "execution-denied",
                ToolResultOutput::ErrorText { .. } => "error-text",
                ToolResultOutput::ErrorJson { .. } => "error-json",
                ToolResultOutput::Content { .. } => "content",
            };
            format!(
                "provider executed tool result output type {output_type} for tool {tool_name} is not supported"
            )
        } else {
            format!("provider executed tool result for tool {tool_name} is not supported")
        };
        warnings.push(Warning::Other { message });
        return Vec::new();
    }
    let value = &raw_value;
    let is_error = matches!(
        output,
        ToolResultOutput::ErrorText { .. }
            | ToolResultOutput::ErrorJson { .. }
            | ToolResultOutput::ExecutionDenied { .. }
    );
    let tool_name = tool_name.as_str();
    let payload_type = value.get("type").and_then(|t| t.as_str());

    let typed = |warnings: &mut Vec<Warning>| -> Option<Value> {
        let mut block = typed_provider_tool_result(
            provider_tool_name,
            tool_name,
            tool_call_id,
            value,
            is_error,
            payload_type,
            warnings,
        )?;
        if matches!(provider_tool_name, "web_fetch" | "web_search")
            && let Some(caller) = tool_caller(provider_options.as_ref(), options_name)
        {
            block["caller"] = caller;
        }
        Some(block)
    };

    let mut blocks = Vec::new();
    if mcp_tool_use_ids.contains(tool_call_id.as_str()) {
        blocks.push(json!({
            "type": "mcp_tool_result",
            "tool_use_id": tool_call_id,
            "is_error": is_error,
            "content": value,
        }));
        // Upstream's MCP branch does not `break`, so the result also goes
        // through the per-tool checks (convert-to-anthropic-prompt.ts:1219-1440):
        // a provider tool name adds its typed block too, tool search takes only
        // `json`, and any other name ends at the unsupported-result warning.
        match provider_tool_name {
            "tool_search_tool_regex" | "tool_search_tool_bm25"
                if matches!(output, ToolResultOutput::ErrorJson { .. }) =>
            {
                warnings.push(Warning::Other {
                    message: format!(
                        "provider executed tool result output type error-json for tool {tool_name} is not supported"
                    ),
                });
            }
            "web_fetch"
            | "web_search"
            | "tool_search_tool_regex"
            | "tool_search_tool_bm25"
            | "advisor" => blocks.extend(typed(warnings)),
            _ => warnings.push(Warning::Other {
                message: format!(
                    "provider executed tool result for tool {tool_name} is not supported"
                ),
            }),
        }
    } else {
        blocks.extend(typed(warnings));
    }

    if let Some(cc) = cache_control {
        for block in &mut blocks {
            block["cache_control"] = cc.clone();
        }
    }
    blocks
}

/// The typed result block for a provider tool, or `None` (with a warning)
/// when the result cannot be sent.
fn typed_provider_tool_result(
    provider_tool_name: &str,
    tool_name: &str,
    tool_call_id: &str,
    value: &Value,
    is_error: bool,
    payload_type: Option<&str>,
    warnings: &mut Vec<Warning>,
) -> Option<Value> {
    let (block_type, content) = match provider_tool_name {
        "code_execution" if is_error => {
            let error = if let Value::String(raw) = value {
                serde_json::from_str(raw).unwrap_or(json!({}))
            } else {
                value.clone()
            };
            let code_error = error.get("type").and_then(Value::as_str)
                == Some("code_execution_tool_result_error");
            (
                if code_error {
                    "code_execution_tool_result"
                } else {
                    "bash_code_execution_tool_result"
                },
                json!({ "type": if code_error { "code_execution_tool_result_error" } else { "bash_code_execution_tool_result_error" }, "error_code": error.get("errorCode").and_then(Value::as_str).unwrap_or("unknown") }),
            )
        }
        "code_execution" => match assistant_code_execution_content(value, payload_type) {
            Some(v) => v,
            None => {
                warnings.push(Warning::Other {
                    message: format!(
                        "provider executed tool result output value is not a valid code execution result for tool {tool_name}"
                    ),
                });
                return None;
            }
        },
        "web_fetch" => (
            "web_fetch_tool_result",
            assistant_web_fetch_content(value, is_error),
        ),
        "web_search" => (
            "web_search_tool_result",
            if is_error {
                json!({ "type": "web_search_tool_result_error", "error_code": result_error_code(value, "unavailable") })
            } else {
                assistant_web_search_content(value)
            },
        ),
        "tool_search_tool_regex" | "tool_search_tool_bm25" => (
            "tool_search_tool_result",
            assistant_tool_search_content(value),
        ),
        "advisor" => (
            "advisor_tool_result",
            assistant_advisor_content(value, payload_type),
        ),
        _ => {
            warnings.push(Warning::Other {
                message: format!(
                    "provider executed tool result for tool {tool_name} is not supported"
                ),
            });
            return None;
        }
    };
    Some(json!({
        "type": block_type,
        "tool_use_id": tool_call_id,
        "content": content,
    }))
}

/// `code_execution` result payload → `(block type, wire content)`.
///
/// The three code-execution tool versions and the bash / text-editor subtools
/// all report through this one caller-facing tool name, so the payload's own
/// `type` selects the block (upstream :928-1064).
fn assistant_code_execution_content(
    value: &Value,
    payload_type: Option<&str>,
) -> Option<(&'static str, Value)> {
    let content = || value.get("content").cloned().unwrap_or(json!([]));
    Some(match payload_type {
        Some("code_execution_result") => (
            "code_execution_tool_result",
            json!({
                "type": "code_execution_result",
                "stdout": field(value, "stdout"),
                "stderr": field(value, "stderr"),
                "return_code": field(value, "return_code"),
                "content": content(),
            }),
        ),
        Some("encrypted_code_execution_result") => (
            "code_execution_tool_result",
            json!({
                "type": "encrypted_code_execution_result",
                "encrypted_stdout": field(value, "encrypted_stdout"),
                "stderr": field(value, "stderr"),
                "return_code": field(value, "return_code"),
                "content": content(),
            }),
        ),
        Some("code_execution_tool_result_error") => (
            "code_execution_tool_result",
            json!({
                "type": "code_execution_tool_result_error",
                "error_code": result_error_code(value, "unknown"),
            }),
        ),
        Some("bash_code_execution_result") => (
            "bash_code_execution_tool_result",
            json!({
                "type": "bash_code_execution_result",
                "stdout": field(value, "stdout"),
                "stderr": field(value, "stderr"),
                "return_code": field(value, "return_code"),
                "content": content(),
            }),
        ),
        Some("bash_code_execution_tool_result_error") => (
            "bash_code_execution_tool_result",
            json!({
                "type": "bash_code_execution_tool_result_error",
                "error_code": result_error_code(value, "unknown"),
            }),
        ),
        // The response side passes text-editor results through unmapped, so
        // they are already in wire shape.
        Some(t) if t.starts_with("text_editor_code_execution") => {
            ("text_editor_code_execution_tool_result", value.clone())
        }
        _ => return None,
    })
}

/// `web_fetch` result payload → wire `web_fetch_tool_result.content`
/// (upstream :1070-1130). Re-snake-cases the camelCase response shape.
fn assistant_web_fetch_content(value: &Value, is_error: bool) -> Value {
    if is_error || value.get("type").and_then(|t| t.as_str()) == Some("web_fetch_tool_result_error")
    {
        return json!({
            "type": "web_fetch_tool_result_error",
            "error_code": result_error_code(value, "unavailable"),
        });
    }
    let inner = value.get("content").cloned().unwrap_or(Value::Null);
    let source = inner.get("source").cloned().unwrap_or(Value::Null);
    let mut content = json!({
        "type": "web_fetch_result",
        "url": field(value, "url"),
        "retrieved_at": field(value, "retrievedAt"),
        "content": {
            "type": "document",
            "title": field(&inner, "title"),
            "source": {
                "type": field(&source, "type"),
                "media_type": field(&source, "mediaType"),
                "data": field(&source, "data"),
            },
        },
    });
    // An absent `citations` stays absent, as `JSON.stringify` drops it.
    if let Some(citations) = inner.get("citations").filter(|value| !value.is_null()) {
        content["content"]["citations"] = citations.clone();
    }
    content
}

/// `web_search` result payload → wire `web_search_tool_result.content`
/// (upstream :1133-1196). A success payload is the result array.
fn assistant_web_search_content(value: &Value) -> Value {
    match value.as_array() {
        Some(results) => Value::Array(
            results
                .iter()
                .map(|r| {
                    json!({
                        "url": field(r, "url"),
                        "title": field(r, "title"),
                        "page_age": field(r, "pageAge"),
                        "encrypted_content": field(r, "encryptedContent"),
                        "type": field(r, "type"),
                    })
                })
                .collect(),
        ),
        None => json!({
            "type": "web_search_tool_result_error",
            "error_code": result_error_code(value, "unavailable"),
        }),
    }
}

/// `tool_search_tool_*` result payload → wire `tool_search_tool_result.content`
/// (upstream :1198-1233).
fn assistant_tool_search_content(value: &Value) -> Value {
    match value.as_array() {
        Some(refs) => json!({
            "type": "tool_search_tool_search_result",
            "tool_references": refs
                .iter()
                .map(|r| json!({ "type": "tool_reference", "tool_name": field(r, "toolName") }))
                .collect::<Vec<_>>(),
        }),
        None => json!({
            "type": "tool_search_tool_result_error",
            "error_code": result_error_code(value, "unavailable"),
        }),
    }
}

/// `advisor` result payload → wire `advisor_tool_result.content`
/// (upstream :1235-1285).
fn assistant_advisor_content(value: &Value, payload_type: Option<&str>) -> Value {
    let with_stop_reason = |mut v: Value| {
        if let Some(sr) = value.get("stopReason")
            && !sr.is_null()
        {
            v["stop_reason"] = sr.clone();
        }
        v
    };
    match payload_type {
        Some("advisor_result") => with_stop_reason(json!({
            "type": "advisor_result",
            "text": field(value, "text"),
        })),
        Some("advisor_redacted_result") => with_stop_reason(json!({
            "type": "advisor_redacted_result",
            "encrypted_content": field(value, "encryptedContent"),
        })),
        _ => json!({
            "type": "advisor_tool_result_error",
            "error_code": result_error_code(value, "unavailable"),
        }),
    }
}

/// Extract provider_options from a tool-result `output` value, mirroring the TS
/// `outputProviderOptions` resolution.
///
/// - When the output object itself carries a `providerOptions` field, it is
///   returned (e.g. `{ type: 'text', value, providerOptions }`).
/// - When the output is a `content` output whose `value` is an array of content
///   parts, the `providerOptions` of the first part that has one is returned.
fn extract_tool_result_output_provider_options(
    output: &ToolResultOutput,
) -> Option<&SharedProviderOptions> {
    match output {
        ToolResultOutput::Text {
            provider_options, ..
        }
        | ToolResultOutput::Json {
            provider_options, ..
        }
        | ToolResultOutput::ErrorText {
            provider_options, ..
        }
        | ToolResultOutput::ErrorJson {
            provider_options, ..
        }
        | ToolResultOutput::ExecutionDenied {
            provider_options, ..
        } => provider_options.as_ref(),
        ToolResultOutput::Content { value } => value.iter().find_map(|part| match part {
            ToolResultContent::Text(part) => part.provider_options.as_ref(),
            ToolResultContent::File(part) => part.provider_options.as_ref(),
            ToolResultContent::Custom { provider_options } => provider_options.as_ref(),
        }),
    }
}

/// Route an inline-bytes file part. `full_media_type` is the resolved
/// `type/subtype` (after byte-sniffing). `title` (from the part's `filename`)
/// is attached to document blocks, matching the TS `title: metadata.title ??
/// part.filename`.
fn route_file_bytes(
    full_media_type: &str,
    bytes: &[u8],
    title: Option<&str>,
    betas: &mut IndexSet<String>,
) -> Result<Value, AiMuxError> {
    use base64::Engine;
    match get_top_level_media_type(full_media_type) {
        "image" => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            Ok(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": full_media_type, "data": b64 }
            }))
        }
        "application" if full_media_type == "application/pdf" => {
            betas.insert(BETA_PDFS.to_string());
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            let mut block = json!({
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": b64 }
            });
            if let Some(t) = title {
                block["title"] = json!(t);
            }
            Ok(block)
        }
        "text" if full_media_type == "text/plain" => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            let mut block = json!({
                "type": "document",
                "source": { "type": "text", "media_type": "text/plain", "data": text }
            });
            if let Some(t) = title {
                block["title"] = json!(t);
            }
            Ok(block)
        }
        _ => Err(AiMuxError::UnsupportedFunctionality(format!(
            "media type: {full_media_type}"
        ))),
    }
}

/// Route a base64-string file part. `b64` is the verbatim base64 string (passed
/// through unchanged for image/pdf sources); `bytes` is the decoded form (used
/// for text/plain and for media-type detection). `title` is attached to
/// document blocks.
fn route_file_base64(
    full_media_type: &str,
    b64: &str,
    bytes: &[u8],
    title: Option<&str>,
    betas: &mut IndexSet<String>,
) -> Result<Value, AiMuxError> {
    match get_top_level_media_type(full_media_type) {
        "image" => Ok(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": full_media_type, "data": b64 }
        })),
        "application" if full_media_type == "application/pdf" => {
            betas.insert(BETA_PDFS.to_string());
            let mut block = json!({
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": b64 }
            });
            if let Some(t) = title {
                block["title"] = json!(t);
            }
            Ok(block)
        }
        "text" if full_media_type == "text/plain" => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            let mut block = json!({
                "type": "document",
                "source": { "type": "text", "media_type": "text/plain", "data": text }
            });
            if let Some(t) = title {
                block["title"] = json!(t);
            }
            Ok(block)
        }
        _ => Err(AiMuxError::UnsupportedFunctionality(format!(
            "media type: {full_media_type}"
        ))),
    }
}

/// Route a URL file part. No byte-sniffing is possible; routing uses the
/// (possibly top-level-only) media type, matching the TS SDK which only checks
/// `mediaType === 'application/pdf'` / `mediaType === 'text/plain'` and the
/// top-level segment for images.
fn route_file_url(
    media_type: &str,
    url: &str,
    betas: &mut IndexSet<String>,
) -> Result<Value, AiMuxError> {
    match get_top_level_media_type(media_type) {
        "image" => Ok(json!({ "type": "image", "source": { "type": "url", "url": url } })),
        "application" if media_type == "application/pdf" => {
            betas.insert(BETA_PDFS.to_string());
            Ok(json!({ "type": "document", "source": { "type": "url", "url": url } }))
        }
        "text" if media_type == "text/plain" => {
            Ok(json!({ "type": "document", "source": { "type": "url", "url": url } }))
        }
        _ => Err(AiMuxError::UnsupportedFunctionality(format!(
            "media type: {media_type}"
        ))),
    }
}

/// Convert an assistant `Reasoning` part into a `thinking` or
/// `redacted_thinking` block, or drop it with a warning. Returns `None` when
/// the part is dropped.
///
/// `cache_control` is rejected on thinking blocks (the validator is invoked
/// with `can_cache = false` so a set value emits a warning and is ignored),
/// matching the TS SDK — thinking blocks are cached implicitly by Anthropic.
fn convert_reasoning_part(
    text: &str,
    provider_options: Option<&SharedProviderOptions>,
    send_reasoning: bool,
    warnings: &mut Vec<Warning>,
    validator: &mut CacheControlValidator,
    options_name: &str,
) -> Option<Value> {
    if !send_reasoning {
        warnings.push(Warning::Other {
            message: "sending reasoning content is disabled for this model".to_string(),
        });
        return None;
    }

    // `redactedData` is read from `providerOptions.anthropic.redactedData`
    // (mirroring the TS `anthropicReasoningMetadataSchema`).
    let reasoning_options = anthropic_options(provider_options, options_name);
    let redacted_data = reasoning_options
        .as_ref()
        .and_then(|a| a.get("redactedData"))
        .and_then(|v| v.as_str());

    let effective_signature = reasoning_options
        .as_ref()
        .and_then(|a| a.get("signature"))
        .and_then(Value::as_str);

    if let Some(sig) = effective_signature {
        // Thinking blocks cannot carry cache_control directly — they are cached
        // implicitly when in previous assistant turns. Validate to emit a
        // helpful warning if a value was set.
        validator.get_cache_control(provider_options, "thinking block", false);
        Some(json!({
            "type": "thinking",
            "thinking": text,
            "signature": sig,
        }))
    } else if let Some(data) = redacted_data {
        // Redacted thinking blocks likewise cannot carry cache_control.
        validator.get_cache_control(provider_options, "redacted thinking block", false);
        Some(json!({
            "type": "redacted_thinking",
            "data": data,
        }))
    } else {
        warnings.push(Warning::Other {
            message: "unsupported reasoning metadata".to_string(),
        });
        None
    }
}

/// Resolve the Anthropic file id from a provider-reference object, mirroring the
/// TS `resolveProviderReference`: the reference is always keyed by the
/// canonical name, whatever the provider is called. Errors when that key is
/// absent, matching the TS `UnsupportedFunctionalityError`.
fn resolve_anthropic_reference(
    reference: &aimux_core::shared::SharedProviderReference,
) -> Result<String, AiMuxError> {
    if let Some(id) = reference.get(CANONICAL) {
        return Ok(id.to_string());
    }
    let providers: Vec<&str> = reference.keys().map(String::as_str).collect();
    Err(AiMuxError::invalid_argument(format!(
        "No provider reference found for provider '{CANONICAL}'. Available providers: {}",
        providers.join(", ")
    )))
}

/// Serialize a unified tool-result payload into the Anthropic content string.
fn resolve_tool_result_output(
    output: &ToolResultOutput,
    betas: &mut IndexSet<String>,
    warnings: &mut Vec<Warning>,
    options_name: &str,
) -> Result<(Value, bool), AiMuxError> {
    let value = match output {
        ToolResultOutput::Text { value, .. } | ToolResultOutput::ErrorText { value, .. } => {
            json!(value)
        }
        ToolResultOutput::Json { value, .. } | ToolResultOutput::ErrorJson { value, .. } => {
            json!(value.to_string())
        }
        ToolResultOutput::ExecutionDenied { reason, .. } => {
            json!(reason.as_deref().unwrap_or("Tool call execution denied."))
        }
        ToolResultOutput::Content { value } => {
            let mut content = Vec::new();
            for part in value {
                match part {
                    ToolResultContent::Text(part) => {
                        content.push(json!({ "type": "text", "text": part.text }))
                    }
                    ToolResultContent::File(part) => {
                        let is_image = get_top_level_media_type(&part.media_type) == "image";
                        let source = match &part.data {
                            FileData::Url { url, .. } => json!({ "type": "url", "url": url }),
                            FileData::Data { data } => {
                                use base64::Engine;
                                let media_type =
                                    aimux_provider_utils::resolve_full_media_type(part)?;
                                if !is_image && media_type != "application/pdf" {
                                    warnings.push(Warning::Other { message: format!("unsupported tool content part type: file with media type: {}", part.media_type) });
                                    continue;
                                }
                                if !is_image {
                                    betas.insert(BETA_PDFS.to_string());
                                }
                                let data = match data {
                                    FileBytes::Binary(bytes) => {
                                        base64::engine::general_purpose::STANDARD.encode(bytes)
                                    }
                                    FileBytes::Base64(data) => data.clone(),
                                };
                                json!({ "type": "base64", "media_type": media_type, "data": data })
                            }
                            data => {
                                let data_type = match data {
                                    FileData::Reference { .. } => "reference",
                                    FileData::Text { .. } => "text",
                                    _ => unreachable!(),
                                };
                                warnings.push(Warning::Other { message: format!("unsupported tool content part type: file with data type: {data_type}") });
                                continue;
                            }
                        };
                        content.push(json!({ "type": if is_image { "image" } else { "document" }, "source": source }));
                    }
                    ToolResultContent::Custom { provider_options } => {
                        if let Some(options) =
                            anthropic_options(provider_options.as_ref(), options_name)
                            && options.get("type").and_then(Value::as_str) == Some("tool-reference")
                        {
                            let mut part = json!({ "type": "tool_reference" });
                            if let Some(tool_name) = options.get("toolName") {
                                part["tool_name"] = tool_name.clone();
                            }
                            content.push(part);
                        } else {
                            warnings.push(Warning::Other {
                                message: "unsupported custom tool content part".to_string(),
                            });
                        }
                    }
                }
            }
            json!(content)
        }
    };
    Ok((
        value,
        matches!(
            output,
            ToolResultOutput::ErrorText { .. } | ToolResultOutput::ErrorJson { .. }
        ),
    ))
}
