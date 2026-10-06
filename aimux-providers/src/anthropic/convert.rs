//! Conversion between `LanguageModelPrompt` and Anthropic API format.
//!
//! This is the single (merged) Anthropic convert module. It provides the full
//! `convertToAnthropicPrompt` implementation with betas, warnings, and
//! `sendReasoning` (mirroring the Vercel AI SDK), plus the legacy
//! `build_request_body` / `parse_stop_reason` helpers used by the language
//! models.
//!
//! Two public prompt-conversion entry points are exposed:
//! - [`convert_prompt_to_anthropic_full`] — the complete conversion, returning
//!   the Anthropic `system` + `messages` shape alongside the beta headers and
//!   warnings produced during conversion. Supports mid-conversation system
//!   messages, trailing-whitespace trimming on the final assistant message,
//!   reasoning/thinking parts, URL & base64 file parts (with PDF beta and
//!   top-level media-type sniffing), and provider-referenced files.
//! - [`convert_prompt_to_anthropic`] — the legacy two-tuple `(system, messages)`
//!   return form used by [`build_request_body`]. It is equivalent to the full
//!   conversion with `send_reasoning = false`, discarding the betas and
//!   warnings.

use std::collections::{BTreeSet, HashSet};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, FunctionTool, ResponseFormat, Tool, ToolChoice};
use aimux_core::shared::{FileBytes, FileData, SharedProviderOptions};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use aimux_provider_utils::{
    MediaTypeData, detect_media_type, get_top_level_media_type, is_full_media_type,
};
use serde_json::{Map, Value, json};

use crate::anthropic::cache_control::CacheControlValidator;
use crate::anthropic::options::{CANONICAL, anthropic_options, anthropic_options_in};
use crate::anthropic::prepare_tools::{AnthropicTool, prepare_tools_with_validator};
use crate::anthropic::tool_name_mapping::ToolNameMapping;

/// Beta header emitted when a PDF file part is present.
const BETA_PDFS: &str = "pdfs-2024-09-25";
/// Beta header emitted when a system message appears mid-conversation.
const BETA_MID_CONVERSATION_SYSTEM: &str = "mid-conversation-system-2026-04-07";
/// Beta header emitted when a provider-referenced file part is present.
const BETA_FILES_API: &str = "files-api-2025-04-14";

/// Result of [`convert_prompt_to_anthropic_full`].
#[derive(Debug, Clone)]
pub struct AnthropicPromptConversion {
    /// Anthropic `system` blocks, or `None` when there is no system prompt.
    pub system: Option<Vec<Value>>,
    /// Anthropic `messages` array.
    pub messages: Vec<Value>,
    /// Beta headers required by the conversion (e.g. `pdfs-2024-09-25`).
    pub betas: BTreeSet<String>,
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
/// Returns `AiMuxError::InvalidArgument` / `UnsupportedFunctionality` when a
/// message part cannot be represented in the Anthropic wire format (e.g. an
/// unresolvable file reference or an unsupported media type).
pub fn convert_prompt_to_anthropic_full_fallible(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    convert_prompt_to_anthropic_full_with_tools(prompt, send_reasoning, &ToolNameMapping::default())
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
/// Same as [`convert_prompt_to_anthropic_full_fallible`]: `AiMuxError::InvalidArgument`
/// / `UnsupportedFunctionality` when a message part cannot be represented in the
/// Anthropic wire format.
pub fn convert_prompt_to_anthropic_full_with_tools(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    convert_prompt_for(prompt, send_reasoning, tool_names, CANONICAL)
}

/// [`convert_prompt_to_anthropic_full_with_tools`] for a provider whose
/// providerOptions key is `options_name`: the part and message options are
/// read from `anthropic` merged with `options_name` (the custom key wins) and
/// the metadata the response side wrote under `options_name` is read back.
///
/// # Errors
///
/// Same as [`convert_prompt_to_anthropic_full_with_tools`].
pub(crate) fn convert_prompt_for(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
    options_name: &str,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    let mut validator = CacheControlValidator::for_options_name(options_name);
    convert_prompt_with_validator(
        prompt,
        send_reasoning,
        tool_names,
        options_name,
        &mut validator,
    )
}

fn convert_prompt_with_validator(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
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
    let mut betas: BTreeSet<String> = BTreeSet::new();
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
                        Some("thinking" | "redacted_thinking")
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

    for msg in prompt {
        let eff = match msg {
            LanguageModelMessage::System {
                content,
                provider_options,
            } => {
                flush(&mut messages, &mut last, &mut acc);
                let cc =
                    validator.get_cache_control(provider_options.as_ref(), "system message", true);
                let blocks = vec![apply_cc(json!({ "type": "text", "text": content }), cc)];
                if !seen_non_system {
                    system.extend(blocks);
                } else {
                    messages.push(json!({ "role": "system", "content": blocks }));
                    betas.insert(BETA_MID_CONVERSATION_SYSTEM.to_string());
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
                        tool_names,
                    )?);
                }
            }
            LanguageModelMessage::Assistant {
                content,
                provider_options,
            } => {
                for (idx, part) in content.iter().enumerate() {
                    let is_last = idx + 1 == content.len();
                    let block = if let AssistantPart::ToolResult(part) = part {
                        convert_assistant_tool_result(
                            part,
                            tool_names,
                            &mcp_tool_use_ids,
                            &mut warnings,
                            validator,
                            is_last,
                            provider_options.as_ref(),
                            options_name,
                        )
                    } else {
                        convert_assistant_part(
                            part,
                            send_reasoning,
                            tool_names,
                            &mut betas,
                            &mut warnings,
                            validator,
                            is_last,
                            provider_options.as_ref(),
                            options_name,
                        )?
                    };
                    if let Some(block) = block {
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

    // Merge any cache_control validation warnings.
    warnings.extend(validator.take_warnings());

    let system_opt = if system.is_empty() {
        None
    } else {
        Some(system)
    };
    Ok(AnthropicPromptConversion {
        system: system_opt,
        messages,
        betas,
        warnings,
    })
}

/// Convert a prompt into the full Anthropic shape.
///
/// Panics on conversion failure. Production paths use the fallible variant
/// [`convert_prompt_to_anthropic_full_fallible`]; this panic wrapper exists
/// only for integration tests under `tests/`. It is `#[doc(hidden)]` and
/// `#[deprecated]` so it neither appears on the public API surface nor can be
/// pulled in by accident (release uses `panic = "abort"`, so reaching a panic
/// here via FFI would kill the host process).
#[doc(hidden)]
#[deprecated(
    since = "0.2.1",
    note = "panics on failure; use convert_prompt_to_anthropic_full_fallible instead (issue #90 R1)"
)]
#[must_use]
pub fn convert_prompt_to_anthropic_full(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
) -> AnthropicPromptConversion {
    convert_prompt_to_anthropic_full_fallible(prompt, send_reasoning)
        .expect("convert_prompt_to_anthropic_full: conversion failed")
}

/// Convert a prompt into the Anthropic `system` + `messages` shape.
///
/// This is the legacy two-tuple return form used by [`build_request_body`]. It
/// is equivalent to [`convert_prompt_to_anthropic_full`] with
/// `send_reasoning = false`, discarding the betas and warnings. Consecutive
/// messages that map to the same effective Anthropic role (`user`/`tool` →
/// `user`, `assistant` → `assistant`) are merged into a single message, matching
/// the SDK behaviour.
#[doc(hidden)]
#[deprecated(
    since = "0.2.1",
    note = "panics on failure; use convert_prompt_to_anthropic_full_fallible instead (issue #90 R1)"
)]
#[must_use]
pub fn convert_prompt_to_anthropic(
    prompt: &LanguageModelPrompt,
) -> (Option<Vec<Value>>, Vec<Value>) {
    match convert_prompt_to_anthropic_full_fallible(prompt, false) {
        Ok(result) => (result.system, result.messages),
        Err(e) => panic!("{}", e),
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
    betas: &mut BTreeSet<String>,
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
    betas: &mut BTreeSet<String>,
    warnings: &mut Vec<Warning>,
    is_last_part: bool,
    message_provider_options: Option<&SharedProviderOptions>,
    options_name: &str,
    tool_names: &ToolNameMapping,
) -> Result<Value, AiMuxError> {
    let ToolResultPart {
        tool_call_id,
        tool_name,
        output,
        provider_options,
        ..
    } = part;

    let (content, is_error) = resolve_tool_result_output(output, betas, warnings, options_name)?;
    let mut block = json!({
        "type": "tool_result",
        "tool_use_id": tool_call_id,
        "content": content,
    });
    if let Some(name) = toolset_name(
        tool_names,
        tool_name,
        provider_options.as_ref(),
        options_name,
    ) {
        block["toolset_name"] = json!(name);
    }
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
    betas: &mut BTreeSet<String>,
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
            if let Some(toolset_name) = toolset_name(
                tool_names,
                tool_name,
                provider_options.as_ref(),
                options_name,
            ) {
                let Some(action) = input_val.get("action").and_then(Value::as_str) else {
                    warnings.push(Warning::Other {
                        message: format!(
                            "toolset tool call for tool {tool_name} is missing the action"
                        ),
                    });
                    return Ok(None);
                };
                let mut block = json!({
                    "type": "tool_use",
                    "id": tool_call_id,
                    "name": action,
                    "toolset_name": toolset_name,
                    "input": input_val,
                });
                block["input"].as_object_mut().unwrap().remove("action");
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

        AssistantPart::ReasoningFile(_) | AssistantPart::Custom(_) => return Ok(None),
        AssistantPart::ToolResult(_) => unreachable!(),
    }))
}

// ── assistant-role tool results (provider-executed) ─────────────────────────

/// Collect the tool call ids that were executed over MCP.
///
/// Their results have to go back as `mcp_tool_result`, so they are indexed
/// before any message is converted. The marker is the one the response side
/// writes on an `mcp_tool_use` block (`stream.rs`):
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

fn toolset_name(
    tool_names: &ToolNameMapping,
    tool_name: &str,
    provider_options: Option<&SharedProviderOptions>,
    options_name: &str,
) -> Option<String> {
    tool_names
        .toolset_name(tool_name)
        .map(str::to_string)
        .or_else(|| {
            anthropic_options(provider_options, options_name).and_then(|options| {
                options
                    .get("toolsetName")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
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
fn field(value: &Value, key: &str) -> Value {
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
/// Returns `None` when the part is skipped.
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
) -> Option<Value> {
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
        warnings.push(Warning::Other {
            message: format!(
                "provider executed tool result output for tool {tool_name} is not supported"
            ),
        });
        return None;
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

    let mut block = if mcp_tool_use_ids.contains(tool_call_id.as_str()) {
        json!({
            "type": "mcp_tool_result",
            "tool_use_id": tool_call_id,
            "is_error": is_error,
            "content": value,
        })
    } else {
        let (block_type, content) = match tool_names.to_provider_tool_name(tool_name) {
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
        json!({
            "type": block_type,
            "tool_use_id": tool_call_id,
            "content": content,
        })
    };

    if matches!(provider_tool_name, "web_fetch" | "web_search")
        && let Some(caller) = tool_caller(provider_options.as_ref(), options_name)
    {
        block["caller"] = caller;
    }
    if let Some(cc) = cache_control {
        block["cache_control"] = cc;
    }
    Some(block)
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
    json!({
        "type": "web_fetch_result",
        "url": field(value, "url"),
        "retrieved_at": field(value, "retrievedAt"),
        "content": {
            "type": "document",
            "title": field(&inner, "title"),
            "citations": field(&inner, "citations"),
            "source": {
                "type": field(&source, "type"),
                "media_type": field(&source, "mediaType"),
                "data": field(&source, "data"),
            },
        },
    })
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
    betas: &mut BTreeSet<String>,
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
    betas: &mut BTreeSet<String>,
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
    betas: &mut BTreeSet<String>,
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
    reference: &std::collections::HashMap<String, String>,
) -> Result<String, AiMuxError> {
    if let Some(id) = reference.get(CANONICAL) {
        return Ok(id.to_string());
    }
    let providers: Vec<&str> = reference.keys().map(String::as_str).collect();
    Err(AiMuxError::InvalidArgument(format!(
        "No provider reference found for provider '{CANONICAL}'. Available providers: {}",
        providers.join(", ")
    )))
}

/// Serialize a unified tool-result payload into the Anthropic content string.
fn resolve_tool_result_output(
    output: &ToolResultOutput,
    betas: &mut BTreeSet<String>,
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

// ── model capabilities & reasoning config ───────────────────────────────────

/// Resolved capabilities for an Anthropic model id, mirroring the TS
/// `getModelCapabilities`.
#[derive(Debug, Clone, Copy)]
struct ModelCapabilities {
    max_output_tokens: u32,
    supports_adaptive_thinking: bool,
    rejects_sampling_parameters: bool,
    supports_xhigh_effort: bool,
    rejects_thinking_disabled_above_high_effort: bool,
    rejects_thinking_disabled: bool,
    rejects_forced_tool_use: bool,
    supports_between_tools_thinking: bool,
    /// Whether the model supports native structured outputs (and therefore
    /// strict tool schemas). Mirrors the TS `supportsStructuredOutput` from
    /// `getModelCapabilities`; `supportsStrictTools` tracks it 1:1 because the
    /// config-level `supportsStrictTools` defaults to `true`.
    supports_structured_output: bool,
    /// Mirrors the TS `isKnownModel`. When `false` and the caller did not set
    /// `maxOutputTokens`, a compatibility warning is emitted noting the
    /// default token limit applied.
    is_known_model: bool,
}

/// Detect whether a model id refers to a legacy Claude model (claude-instant,
/// claude-2, claude-3 — but *not* `claude-3-haiku`, which is matched earlier).
/// Mirrors the TS regex `/claude-(?:instant(?:-|$)|v?2(?=$|[-.:])|3(?=$|[-.]))/`.
fn is_legacy_claude(model_id: &str) -> bool {
    let rest = match model_id.find("claude-") {
        Some(i) => &model_id[i + "claude-".len()..],
        None => return false,
    };
    if rest == "instant" || rest.starts_with("instant-") {
        return true;
    }
    let two_part = rest.strip_prefix('v').unwrap_or(rest);
    if two_part == "2"
        || two_part.starts_with("2-")
        || two_part.starts_with("2.")
        || two_part.starts_with("2:")
    {
        return true;
    }
    if rest == "3" || rest.starts_with("3-") || rest.starts_with("3.") {
        return true;
    }
    false
}

/// Returns the capabilities for an Anthropic model id, mirroring the TS
/// `getModelCapabilities`. The order of the `contains` checks matters.
fn get_model_capabilities(model_id: &str) -> ModelCapabilities {
    fn caps(
        max_output_tokens: u32,
        supports_adaptive_thinking: bool,
        rejects_sampling_parameters: bool,
        supports_xhigh_effort: bool,
        rejects_thinking_disabled_above_high_effort: bool,
        supports_structured_output: bool,
        is_known_model: bool,
    ) -> ModelCapabilities {
        ModelCapabilities {
            max_output_tokens,
            supports_adaptive_thinking,
            rejects_sampling_parameters,
            supports_xhigh_effort,
            rejects_thinking_disabled_above_high_effort,
            rejects_thinking_disabled: false,
            rejects_forced_tool_use: false,
            supports_between_tools_thinking: false,
            supports_structured_output,
            is_known_model,
        }
    }

    if let Some((_, suffix)) = model_id.split_once("claude-opus-5") {
        let mut result = caps(128000, true, true, true, true, true, true);
        result.rejects_thinking_disabled = suffix.starts_with("-5");
        result.rejects_forced_tool_use = result.rejects_thinking_disabled;
        result
    } else if let Some((_, suffix)) = model_id.split_once("claude-fable-5") {
        let mut result = caps(128000, true, true, true, false, true, true);
        result.rejects_thinking_disabled = true;
        result.rejects_forced_tool_use = suffix.starts_with("-1");
        result
    } else if let Some((_, suffix)) = model_id.split_once("claude-sonnet-5") {
        let mut result = caps(128000, true, true, true, false, true, true);
        result.supports_between_tools_thinking = suffix.starts_with("-5");
        result.rejects_thinking_disabled_above_high_effort = result.supports_between_tools_thinking;
        result.rejects_thinking_disabled = result.supports_between_tools_thinking;
        result.rejects_forced_tool_use = result.supports_between_tools_thinking;
        result
    } else if model_id.contains("claude-opus-4-8") || model_id.contains("claude-opus-4-7") {
        caps(128000, true, true, true, false, true, true)
    } else if model_id.contains("claude-sonnet-4-6") || model_id.contains("claude-opus-4-6") {
        caps(128000, true, false, false, false, true, true)
    } else if model_id.contains("claude-sonnet-4-5")
        || model_id.contains("claude-opus-4-5")
        || model_id.contains("claude-haiku-4-5")
    {
        caps(64000, false, false, false, false, true, true)
    } else if model_id.contains("claude-opus-4-1") {
        caps(32000, false, false, false, false, true, true)
    } else if model_id.contains("claude-sonnet-4-") {
        caps(64000, false, false, false, false, false, true)
    } else if model_id.contains("claude-opus-4-") {
        caps(32000, false, false, false, false, false, true)
    } else if model_id.contains("claude-3-haiku") {
        caps(4096, false, false, false, false, false, true)
    } else if is_legacy_claude(model_id) {
        caps(4096, false, false, false, false, false, false)
    } else if model_id.contains("claude-") {
        caps(128000, true, true, true, true, true, false)
    } else {
        caps(4096, false, false, false, false, false, false)
    }
}

/// The Anthropic thinking config + optional effort resolved from a top-level
/// reasoning level, mirroring the TS `resolveAnthropicReasoningConfig` return.
struct ReasoningConfig {
    thinking: Value,
    effort: Option<String>,
}

/// Map a reasoning level to a provider effort string, pushing a compatibility
/// warning when the level maps to a different string. Mirrors the TS
/// `mapReasoningToProviderEffort`.
fn map_reasoning_to_effort(
    reasoning: ReasoningEffort,
    supports_xhigh: bool,
    warnings: &mut Vec<Warning>,
) -> Option<String> {
    let level = reasoning.to_string();
    let mapped = match reasoning {
        ReasoningEffort::Minimal => Some("low"),
        ReasoningEffort::Low => Some("low"),
        ReasoningEffort::Medium => Some("medium"),
        ReasoningEffort::High => Some("high"),
        ReasoningEffort::Xhigh => {
            if supports_xhigh {
                Some("xhigh")
            } else {
                Some("max")
            }
        }
        ReasoningEffort::ProviderDefault | ReasoningEffort::None => None,
    };

    let mapped = match mapped {
        Some(m) => m,
        None => {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".to_string(),
                details: Some(format!(
                    "reasoning \"{level}\" is not supported by this model."
                )),
            });
            return None;
        }
    };

    if mapped != level {
        warnings.push(Warning::Compatibility {
            feature: "reasoning".to_string(),
            details: Some(format!(
                "reasoning \"{level}\" is not directly supported by this model. mapped to effort \"{mapped}\"."
            )),
        });
    }

    Some(mapped.to_string())
}

/// Default reasoning budget percentages (of max output tokens), mirroring the
/// TS `DEFAULT_REASONING_BUDGET_PERCENTAGES`.
fn reasoning_budget_percentage(reasoning: ReasoningEffort) -> Option<f64> {
    match reasoning {
        ReasoningEffort::Minimal => Some(0.02),
        ReasoningEffort::Low => Some(0.10),
        ReasoningEffort::Medium => Some(0.30),
        ReasoningEffort::High => Some(0.60),
        ReasoningEffort::Xhigh => Some(0.90),
        ReasoningEffort::ProviderDefault | ReasoningEffort::None => None,
    }
}

/// Map a reasoning level to an absolute token budget, clamped to a minimum of
/// 1024 and the model's max output tokens. Mirrors the TS
/// `mapReasoningToProviderBudget`.
fn map_reasoning_to_budget(
    reasoning: ReasoningEffort,
    max_output_tokens: u32,
    warnings: &mut Vec<Warning>,
) -> Option<u32> {
    let pct = match reasoning_budget_percentage(reasoning) {
        Some(p) => p,
        None => {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".to_string(),
                details: Some(format!(
                    "reasoning \"{reasoning}\" is not supported by this model."
                )),
            });
            return None;
        }
    };
    let raw = (max_output_tokens as f64 * pct).round() as u32;
    Some(max_output_tokens.min(1024.max(raw)))
}

/// Resolve a top-level reasoning level into an Anthropic thinking config +
/// optional effort, mirroring the TS `resolveAnthropicReasoningConfig`.
///
/// Returns `None` for `ProviderDefault` (no config). `None` reasoning maps to a
/// `disabled` thinking config. Other levels map to adaptive thinking (with an
/// effort) or budget-based `enabled` thinking, depending on
/// `supports_adaptive_thinking`.
fn resolve_anthropic_reasoning_config(
    reasoning: ReasoningEffort,
    supports_adaptive_thinking: bool,
    supports_xhigh_effort: bool,
    max_output_tokens_for_model: u32,
    warnings: &mut Vec<Warning>,
) -> Option<ReasoningConfig> {
    if reasoning == ReasoningEffort::ProviderDefault {
        return None;
    }
    if reasoning == ReasoningEffort::None {
        return Some(ReasoningConfig {
            thinking: json!({ "type": "disabled" }),
            effort: None,
        });
    }

    if supports_adaptive_thinking {
        let effort = map_reasoning_to_effort(reasoning, supports_xhigh_effort, warnings)?;
        return Some(ReasoningConfig {
            thinking: json!({ "type": "adaptive", "display": "summarized" }),
            effort: Some(effort),
        });
    }

    let budget_tokens = map_reasoning_to_budget(reasoning, max_output_tokens_for_model, warnings)?;
    Some(ReasoningConfig {
        thinking: json!({ "type": "enabled", "budgetTokens": budget_tokens }),
        effort: None,
    })
}

/// Result of building an Anthropic request body, including warnings and the
/// beta headers required by the request.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub(crate) uses_json_response_tool: bool,
    pub body: Value,
    pub warnings: Vec<Warning>,
    /// Beta headers (e.g. `code-execution-2025-08-25`, `mcp-client-2025-04-04`)
    /// that must be sent on the request via the `anthropic-beta` header.
    pub betas: BTreeSet<String>,
}

fn parse_anthropic_option_object(shape: &str, value: &Value) -> Result<Value, ()> {
    let object = value.as_object().ok_or(())?;
    let parsed = crate::openai::convert::parse_option_fields(object, "anthropic", |key, value| {
        let kind = match (shape, key) {
            ("options", "sendReasoning" | "disableParallelToolUse" | "toolStreaming") => "bool",
            ("options", "structuredOutputMode") => "outputFormat|jsonTool|auto",
            ("options", "thinking") => "thinking",
            ("options", "cacheControl") => "cache",
            ("options", "metadata") => "metadata",
            ("options", "mcpServers") => "[]mcp",
            ("options", "container") => "container",
            ("options", "effort") => "low|medium|high|xhigh|max",
            ("options", "taskBudget") => "taskBudget",
            ("options", "speed") | ("fallback", "speed") => "fast|standard",
            ("options", "serviceTier") => "auto|standard_only",
            ("options", "inferenceGeo") => "us|global",
            ("options", "fallbacks") => "fallbacks",
            ("options", "anthropicBeta") => "[]string",
            ("options", "safeguards") => "[]safeguard",
            ("options", "compaction") => "compaction",
            ("options", "contextManagement") => "context",
            ("thinking", "type") => "adaptive|enabled|disabled|between_tools",
            ("thinking", "display")
                if object.get("type").and_then(Value::as_str) == Some("adaptive") =>
            {
                "omitted|summarized|updates"
            }
            ("thinking", "budgetTokens")
                if object.get("type").and_then(Value::as_str) == Some("enabled") =>
            {
                "number"
            }
            ("thinking", "blockBinding")
                if object
                    .get("type")
                    .is_none_or(|t| t.as_str() == Some("adaptive")) =>
            {
                "binding"
            }
            ("binding", "prefixMismatchBehavior") => "error|drop_block",
            ("cache", "type") => "ephemeral",
            ("cache", "ttl") => "5m|1h",
            ("metadata", "userId") => "string",
            ("mcp", "type") => "url",
            ("mcp", "name" | "url") => "string",
            ("mcp", "authorizationToken") => "?string",
            ("mcp", "toolConfiguration") => "?toolConfiguration",
            ("toolConfiguration", "enabled") => "?bool",
            ("toolConfiguration", "allowedTools") => "?[]string",
            ("container", "id") | ("skill", "version") => "string",
            ("container", "skills") => "[]skill",
            ("skill", "type") => "anthropic|custom",
            ("skill", "skillId")
                if object.get("type").and_then(Value::as_str) == Some("anthropic") =>
            {
                "string"
            }
            ("skill", "providerReference")
                if object.get("type").and_then(Value::as_str) == Some("custom") =>
            {
                "stringRecord"
            }
            ("taskBudget", "type") => "tokens",
            ("taskBudget", "total") => "total",
            ("taskBudget", "remaining") => "remaining",
            ("fallback", "model") => "string",
            ("fallback", "max_tokens") => "integer",
            ("fallback", "thinking" | "output_config") | ("safeguard", "classifierContext") => {
                "record"
            }
            ("safeguard", "type") => "dangerous_tool_use",
            ("compaction", "type") => "summarize",
            ("compaction", "instructions") => "string",
            ("edit", "instructions")
                if object.get("type").and_then(Value::as_str) == Some("compact_20260112") =>
            {
                "string"
            }
            ("context", "edits") => "[]edit",
            ("edit", "type") => "clear_tool_uses_20250919|clear_thinking_20251015|compact_20260112",
            ("edit", "trigger")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "trigger"
            }
            ("edit", "trigger")
                if object.get("type").and_then(Value::as_str) == Some("compact_20260112") =>
            {
                "inputTokens"
            }
            ("edit", "clearAtLeast")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "inputTokens"
            }
            ("edit", "keep")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "toolUses"
            }
            ("edit", "keep")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_thinking_20251015") =>
            {
                "keepThinking"
            }
            ("edit", "clearToolInputs")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "bool"
            }
            ("edit", "excludeTools")
                if object.get("type").and_then(Value::as_str)
                    == Some("clear_tool_uses_20250919") =>
            {
                "[]string"
            }
            ("edit", "pauseAfterCompaction")
                if object.get("type").and_then(Value::as_str) == Some("compact_20260112") =>
            {
                "bool"
            }
            ("trigger", "type") => "input_tokens|tool_uses",
            ("inputTokens", "type") => "input_tokens",
            ("toolUses", "type") => "tool_uses",
            ("thinkingTurns", "type") => "thinking_turns",
            ("trigger" | "inputTokens" | "toolUses" | "thinkingTurns", "value") => "number",
            _ => return None,
        };
        Some(parse_anthropic_option_value(kind, value))
    })
    .map_err(|_| ())?;
    let required: &[&str] = match shape {
        "thinking" if !object.contains_key("type") => &["blockBinding"],
        "thinking" | "cache" | "compaction" | "safeguard" | "edit" => &["type"],
        "binding" => &["prefixMismatchBehavior"],
        "mcp" => &["type", "name", "url"],
        "skill" if object.get("type").and_then(Value::as_str) == Some("custom") => {
            &["type", "providerReference"]
        }
        "skill" => &["type", "skillId"],
        "taskBudget" => &["type", "total"],
        "fallback" => &["model"],
        "context" => &["edits"],
        "trigger" | "inputTokens" | "toolUses" | "thinkingTurns" => &["type", "value"],
        _ => &[],
    };
    if required.iter().any(|key| !parsed.contains_key(*key)) {
        return Err(());
    }
    Ok(Value::Object(parsed))
}

fn parse_anthropic_option_value(kind: &str, value: &Value) -> Result<Value, ()> {
    if let Some(kind) = kind.strip_prefix('?') {
        return if value.is_null() {
            Ok(Value::Null)
        } else {
            parse_anthropic_option_value(kind, value)
        };
    }
    if let Some(kind) = kind.strip_prefix("[]") {
        return value
            .as_array()
            .ok_or(())?
            .iter()
            .map(|v| parse_anthropic_option_value(kind, v))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    let valid = match kind {
        "bool" => value.is_boolean(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0),
        "total" => value
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && (20000.0..=9_007_199_254_740_991.0).contains(&n)),
        "remaining" => value
            .as_f64()
            .is_some_and(|n| n.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(&n)),
        "record" => value.is_object(),
        "stringRecord" => value
            .as_object()
            .is_some_and(|o| o.values().all(Value::is_string)),
        "fallbacks" => {
            return if value.as_str() == Some("default") {
                Ok(value.clone())
            } else {
                parse_anthropic_option_value("[]fallback", value)
            };
        }
        "keepThinking" => {
            return if value.as_str() == Some("all") {
                Ok(value.clone())
            } else {
                parse_anthropic_option_object("thinkingTurns", value)
            };
        }
        "thinking" | "binding" | "cache" | "metadata" | "mcp" | "toolConfiguration"
        | "container" | "skill" | "taskBudget" | "fallback" | "safeguard" | "compaction"
        | "context" | "edit" | "trigger" | "inputTokens" | "toolUses" | "thinkingTurns" => {
            return parse_anthropic_option_object(kind, value);
        }
        _ => value
            .as_str()
            .is_some_and(|s| kind.split('|').any(|allowed| allowed == s)),
    };
    if valid { Ok(value.clone()) } else { Err(()) }
}

/// What differs between the hosts of the Messages API as far as the request
/// body goes: which providerOptions key the caller's options live under (read
/// in addition to the canonical one) and which tool/output features the host
/// accepts.
#[derive(Debug, Clone)]
pub(crate) struct RequestProfile {
    /// The custom providerOptions key; `anthropic` is always read too.
    pub(crate) options_name: String,
    /// Structured outputs and their beta header (Vertex has neither).
    pub(crate) supports_native_structured_output: bool,
    /// `strict` on tool definitions (Vertex rejects it).
    pub(crate) supports_strict_tools: bool,
}

impl Default for RequestProfile {
    /// The first-party API under its canonical name.
    fn default() -> Self {
        Self {
            options_name: CANONICAL.to_string(),
            supports_native_structured_output: true,
            supports_strict_tools: true,
        }
    }
}

/// Read a value from the Anthropic options of a call (`anthropic` merged with
/// the profile's custom key).
fn anthropic_option(
    options: &Option<SharedProviderOptions>,
    profile: &RequestProfile,
    key: &str,
) -> Option<Value> {
    anthropic_options_in(options.as_ref(), &profile.options_name).and_then(|o| o.get(key).cloned())
}

/// Recursively remove `null`-valued fields from JSON objects (mirroring the
/// TS `JSON.stringify` behaviour of dropping `undefined`). Array elements are
/// preserved as-is. Used to strip absent provider-tool args (e.g. `max_uses`
/// on a web_search tool with no `maxUses`) so the request body matches the
/// TS snapshots.
fn strip_null_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for (_, v) in map.iter_mut() {
                strip_null_fields(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_null_fields(v);
            }
        }
        _ => {}
    }
}

/// Build the Anthropic request body (without warnings). Returns an error when
/// provider-option resolution fails (e.g. a custom skill provider reference
/// that does not include the `anthropic` key).
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when provider-option resolution
/// fails (e.g. a custom skill reference missing the `anthropic` key).
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<Value, AiMuxError> {
    Ok(build_request_body_with_warnings(model_id, options, stream)?.body)
} // ── Request-body construction (split into helpers, issue M11) ───────────────

/// Strip temperature / topK / topP when the model rejects sampling parameters
/// (with a compatibility warning per stripped value).
fn strip_anthropic_sampling_params(
    options: &CallOptions,
    model_id: &str,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut temperature = options.temperature;
    for (present, feature) in [
        (options.frequency_penalty.is_some(), "frequencyPenalty"),
        (options.presence_penalty.is_some(), "presencePenalty"),
        (options.seed.is_some(), "seed"),
    ] {
        if present {
            warnings.push(Warning::Unsupported {
                feature: feature.to_string(),
                details: None,
            });
        }
    }
    if let Some(value) = temperature
        && !(0.0..=1.0).contains(&value)
    {
        let (limit, details) = if value > 1.0 {
            (
                1.0,
                format!("{value} exceeds anthropic maximum of 1.0. clamped to 1.0"),
            )
        } else {
            (
                0.0,
                format!("{value} is below anthropic minimum of 0. clamped to 0"),
            )
        };
        temperature = Some(limit);
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some(details),
        });
    }
    let mut top_p = options.top_p;
    let mut top_k = options.top_k;

    if caps.rejects_sampling_parameters {
        if temperature.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "temperature".to_string(),
                details: Some(format!(
                    "temperature is not supported by {model_id} and will be ignored"
                )),
            });
            temperature = None;
        }
        if top_k.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: Some(format!(
                    "topK is not supported by {model_id} and will be ignored"
                )),
            });
            top_k = None;
        }
        if top_p.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topP".to_string(),
                details: Some(format!(
                    "topP is not supported by {model_id} and will be ignored"
                )),
            });
            top_p = None;
        }
    }

    (temperature, top_p, top_k)
}

/// Resolve thinking config + optional effort from provider options and the
/// top-level `reasoning` level.
///
/// `providerOptions.anthropic.thinking` / `.effort` take precedence over the
/// top-level `reasoning`; the top-level mapping only runs when `effort` is not
/// already set by provider options (TS L426-445). Also lowers xhigh/max to
/// high when the model rejects disabling thinking above high effort
/// (TS L451-464).
fn resolve_anthropic_thinking(
    model_id: &str,
    options: &CallOptions,
    profile: &RequestProfile,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) -> (Option<Value>, Option<String>) {
    let mut thinking_config: Option<Value> =
        anthropic_option(&options.provider_options, profile, "thinking");
    let mut effort: Option<String> = anthropic_option(&options.provider_options, profile, "effort")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));

    if let Some(reasoning) = options.reasoning
        && reasoning.is_custom()
        && effort.is_none()
        && let Some(rc) = resolve_anthropic_reasoning_config(
            reasoning,
            caps.supports_adaptive_thinking,
            caps.supports_xhigh_effort,
            caps.max_output_tokens,
            warnings,
        )
    {
        if thinking_config.is_none() {
            thinking_config = Some(rc.thinking);
        }
        if let Some(eff) = rc.effort {
            let is_disabled = thinking_config
                .as_ref()
                .and_then(|t| t.get("type"))
                .and_then(|t| t.as_str())
                == Some("disabled");
            if !is_disabled {
                effort = Some(eff);
            }
        }
    }

    if options.reasoning == Some(ReasoningEffort::None)
        && anthropic_option(&options.provider_options, profile, "thinking").is_none()
        && anthropic_option(&options.provider_options, profile, "effort").is_none()
        && caps.rejects_thinking_disabled
    {
        if caps.supports_between_tools_thinking {
            thinking_config = Some(json!({ "type": "between_tools" }));
        } else {
            thinking_config = None;
            effort = Some("low".to_string());
            warnings.push(Warning::Compatibility { feature: "reasoning".to_string(), details: Some(format!("reasoning 'none' is not supported by {model_id}; it always uses adaptive thinking. Using effort 'low' to minimize thinking instead.")) });
        }
    }

    if caps.rejects_thinking_disabled {
        match thinking_config
            .as_ref()
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
        {
            Some("disabled") => {
                let details = if caps.supports_between_tools_thinking {
                    thinking_config = Some(json!({ "type": "between_tools" }));
                    format!(
                        "thinking cannot be disabled for {model_id}. Using 'between_tools' thinking, the lowest thinking setting, instead."
                    )
                } else {
                    thinking_config = None;
                    format!(
                        "thinking cannot be disabled for {model_id}; it always uses adaptive thinking. The thinking setting has been removed. Lower 'effort' to reduce thinking."
                    )
                };
                warnings.push(Warning::Unsupported {
                    feature: "providerOptions.anthropic.thinking".to_string(),
                    details: Some(details),
                });
            }
            Some("enabled") => {
                thinking_config = Some(json!({ "type": "adaptive" }));
                warnings.push(Warning::Unsupported { feature: "providerOptions.anthropic.thinking".to_string(), details: Some(format!("budget-based thinking is not supported by {model_id}; it always uses adaptive thinking. Using adaptive thinking instead. Use 'effort' to control how much the model thinks.")) });
            }
            _ => {}
        }
    }
    if thinking_config
        .as_ref()
        .and_then(|v| v.get("type"))
        .and_then(Value::as_str)
        == Some("between_tools")
        && matches!(effort.as_deref(), Some("xhigh" | "max"))
    {
        warnings.push(Warning::Unsupported { feature: "providerOptions.anthropic.effort".to_string(), details: Some(format!("effort '{}' is not supported with 'between_tools' thinking. The effort has been lowered to 'high'.", effort.as_deref().unwrap_or_default())) });
        effort = Some("high".to_string());
    }

    // Newer models only allow disabling thinking at effort ≤ high; lower the
    // effort to 'high' with a warning (TS L451-464).
    if caps.rejects_thinking_disabled_above_high_effort {
        let is_disabled = thinking_config
            .as_ref()
            .and_then(|t| t.get("type"))
            .and_then(|t| t.as_str())
            == Some("disabled");
        if is_disabled && (effort.as_deref() == Some("xhigh") || effort.as_deref() == Some("max")) {
            warnings.push(Warning::Unsupported {
                feature: "providerOptions.anthropic.effort".to_string(),
                details: Some(format!(
                    "effort '{}' is not supported by {} when thinking is disabled. The effort has been lowered to 'high'.",
                    effort.as_deref().unwrap_or(""),
                    model_id
                )),
            });
            effort = Some("high".to_string());
        }
    }

    (thinking_config, effort)
}

/// Derived fields from the resolved thinking config: the type string, whether
/// thinking must be forwarded (enabled/adaptive/disabled — some models default
/// thinking on, so omitting `disabled` would leave it enabled), the budget
/// (enabled only), and the display value (adaptive only).
fn derive_anthropic_thinking(
    thinking_config: &Option<Value>,
) -> (Option<String>, bool, Option<u32>, Option<Value>) {
    let thinking_type: Option<String> = thinking_config
        .as_ref()
        .and_then(|t| t.get("type"))
        .and_then(|t| t.as_str())
        .map(std::string::ToString::to_string);

    let is_thinking = matches!(
        thinking_type.as_deref(),
        Some("enabled" | "adaptive" | "between_tools")
    );
    let send_thinking = is_thinking || thinking_type.as_deref() == Some("disabled");

    let thinking_budget: Option<u32> = if thinking_type.as_deref() == Some("enabled") {
        thinking_config
            .as_ref()
            .and_then(|t| t.get("budgetTokens"))
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32)
    } else {
        None
    };
    let thinking_display: Option<Value> = if thinking_type.as_deref() == Some("adaptive") {
        thinking_config
            .as_ref()
            .and_then(|t| t.get("display"))
            .cloned()
    } else {
        None
    };

    (
        thinking_type,
        send_thinking,
        thinking_budget,
        thinking_display,
    )
}

/// Insert the `thinking` / `output_config` fields into the body.
fn insert_anthropic_thinking(
    body: &mut Value,
    thinking_type: &Option<String>,
    send_thinking: bool,
    thinking_budget: &Option<u32>,
    thinking_display: &Option<Value>,
    effort: &Option<String>,
) {
    if send_thinking && let Some(tt) = thinking_type {
        let mut thinking_obj = json!({ "type": tt });
        if let Some(b) = thinking_budget {
            thinking_obj["budget_tokens"] = json!(b);
        }
        if let Some(d) = thinking_display {
            thinking_obj["display"] = d.clone();
        }
        body["thinking"] = thinking_obj;
    }

    if let Some(eff) = effort {
        body["output_config"] = json!({ "effort": eff });
    }
}

/// Thinking-enabled post-processing (TS L651-696): default budget warning,
/// sampling-parameter stripping, and `max_tokens` adjustment. Returns the
/// adjusted `max_tokens` (unchanged when thinking is not enabled).
#[allow(clippy::too_many_arguments)]
fn apply_anthropic_thinking_post_processing(
    body: &mut Value,
    thinking_type: &Option<String>,
    thinking_budget: &mut Option<u32>,
    max_tokens: u32,
    temperature: &mut Option<f64>,
    top_k: &mut Option<f64>,
    top_p: &mut Option<f64>,
    warnings: &mut Vec<Warning>,
) -> u32 {
    let is_thinking = matches!(
        thinking_type.as_deref(),
        Some("enabled" | "adaptive" | "between_tools")
    );
    if !is_thinking {
        return max_tokens;
    }

    if thinking_type.as_deref() == Some("enabled") && thinking_budget.is_none() {
        warnings.push(Warning::Compatibility {
            feature: "extended thinking".to_string(),
            details: Some(
                "thinking budget is required when thinking is enabled. using default budget of 1024 tokens.".to_string(),
            ),
        });
        *thinking_budget = Some(1024);
        // Mirrors the original implementation: the default budget must also be
        // written into the `thinking` body field, not just added to max_tokens
        // (audit finding — default budget was dropped during the M11 split).
        if let Some(thinking_obj) = body.get_mut("thinking") {
            thinking_obj["budget_tokens"] = json!(1024);
        }
    }

    if temperature.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some("temperature is not supported when thinking is enabled".to_string()),
        });
        *temperature = None;
    }
    if top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: Some("topK is not supported when thinking is enabled".to_string()),
        });
        *top_k = None;
    }
    if top_p.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topP".to_string(),
            details: Some("topP is not supported when thinking is enabled".to_string()),
        });
        *top_p = None;
    }

    max_tokens.saturating_add(thinking_budget.unwrap_or(0))
}

/// Insert the surviving sampling params + stop sequences into the body.
fn insert_anthropic_sampling(
    body: &mut Value,
    temperature: Option<f64>,
    top_p: Option<f64>,
    top_k: Option<f64>,
    options: &CallOptions,
) {
    if let Some(temp) = temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
    if let Some(k) = top_k {
        body["top_k"] = json!(k);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop_sequences"] = json!(stop);
    }
}

/// Map user tools to `tools` / `tool_choice`, delegating to
/// `prepare_tools_with_provider` (provider-defined tools, required beta
/// headers, and tool warnings).
#[allow(clippy::too_many_arguments)]
fn apply_anthropic_tools(
    body: &mut Value,
    options: &CallOptions,
    profile: &RequestProfile,
    stream: bool,
    json_schema: Option<&Value>,
    caps: &ModelCapabilities,
    betas: &mut BTreeSet<String>,
    warnings: &mut Vec<Warning>,
    cache_validator: &mut CacheControlValidator,
) {
    let disable_parallel_tool_use = json_schema.is_some()
        || anthropic_option(&options.provider_options, profile, "disableParallelToolUse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let default_eager_input_streaming = stream
        && anthropic_option(&options.provider_options, profile, "toolStreaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

    let mut anthropic_tools: Vec<AnthropicTool> = match &options.tools {
        Some(tools) => tools
            .iter()
            .map(|t| match t {
                Tool::Function(ft) => AnthropicTool::Function(ft.clone()),
                Tool::Provider(pt) => AnthropicTool::Provider {
                    id: pt.id.clone(),
                    name: pt.name.clone(),
                    args: pt.args.clone(),
                },
            })
            .collect(),
        None => Vec::new(),
    };

    if let Some(schema) = json_schema {
        anthropic_tools.push(AnthropicTool::Function(
            FunctionTool::new("json", schema.clone())
                .with_description("Respond with a JSON object."),
        ));
    }
    let requested_choice = if json_schema.is_some() {
        Some(&ToolChoice::Required)
    } else {
        options.tool_choice.as_ref()
    };
    let tool_choice = if caps.rejects_forced_tool_use
        && !anthropic_tools.is_empty()
        && matches!(
            requested_choice,
            Some(ToolChoice::Required | ToolChoice::Tool { .. })
        ) {
        warnings.push(Warning::Unsupported {
            feature: "toolChoice".to_string(),
            details: Some(match requested_choice {
                Some(ToolChoice::Required) => "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.".to_string(),
                Some(ToolChoice::Tool { tool_name }) => format!("toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the '{tool_name}' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made."),
                _ => unreachable!(),
            }),
        });
        Some(&ToolChoice::Auto)
    } else {
        requested_choice
    };
    let prepared = prepare_tools_with_validator(
        if options.tools.is_some() || json_schema.is_some() {
            Some(&anthropic_tools)
        } else {
            None
        },
        tool_choice,
        disable_parallel_tool_use,
        json_schema.is_none()
            && caps.supports_structured_output
            && profile.supports_native_structured_output,
        caps.supports_structured_output && profile.supports_strict_tools,
        default_eager_input_streaming,
        &profile.options_name,
        cache_validator,
    );

    warnings.extend(prepared.tool_warnings);
    betas.extend(prepared.betas);

    if let Some(tool_defs) = prepared.tools {
        // Strip null-valued fields from tool definitions so the serialized
        // request body matches the TS behaviour (JSON.stringify drops
        // `undefined` args on provider-defined tools such as web_search).
        let mut defs = tool_defs;
        if caps.rejects_forced_tool_use
            && let Some(ToolChoice::Tool { tool_name }) = requested_choice
        {
            defs.retain(|tool| {
                tool.get("name").and_then(Value::as_str) == Some(tool_name.as_str())
            });
        }
        for def in defs.iter_mut() {
            strip_null_fields(def);
        }
        body["tools"] = json!(defs);
    }
    if let Some(mut tool_choice) = prepared.tool_choice {
        if json_schema.is_none()
            && let Some(value) =
                anthropic_option(&options.provider_options, profile, "disableParallelToolUse")
                    .filter(Value::is_boolean)
        {
            tool_choice["disable_parallel_tool_use"] = value;
        }
        body["tool_choice"] = tool_choice;
    }
}

/// `providerOptions.anthropic.mcpServers` → `mcp_servers` + the
/// `mcp-client-2025-04-04` beta header.
fn append_anthropic_mcp_servers(
    body: &mut Value,
    options: &CallOptions,
    profile: &RequestProfile,
    betas: &mut BTreeSet<String>,
) {
    let Some(mcp) = anthropic_option(&options.provider_options, profile, "mcpServers") else {
        return;
    };
    let Some(arr) = mcp.as_array() else {
        return;
    };
    if arr.is_empty() {
        return;
    }

    let mapped: Vec<Value> = arr
        .iter()
        .map(|server| {
            let mut o = Map::new();
            if let Some(t) = server.get("type") {
                o.insert("type".to_string(), t.clone());
            }
            if let Some(n) = server.get("name") {
                o.insert("name".to_string(), n.clone());
            }
            if let Some(u) = server.get("url") {
                o.insert("url".to_string(), u.clone());
            }
            if let Some(at) = server.get("authorizationToken") {
                o.insert("authorization_token".to_string(), at.clone());
            }
            if let Some(tc) = server
                .get("toolConfiguration")
                .filter(|value| !value.is_null())
            {
                let mut tc_obj = Map::new();
                if let Some(at) = tc.get("allowedTools") {
                    tc_obj.insert("allowed_tools".to_string(), at.clone());
                }
                if let Some(en) = tc.get("enabled") {
                    tc_obj.insert("enabled".to_string(), en.clone());
                }
                o.insert("tool_configuration".to_string(), Value::Object(tc_obj));
            }
            Value::Object(o)
        })
        .collect();
    body["mcp_servers"] = json!(mapped);
    betas.insert("mcp-client-2025-04-04".to_string());
}

/// `providerOptions.anthropic.container` — programmatic tool calling (string
/// id) or agent skills (object with id + skills). Skills require the code
/// execution beta headers; returns an error when a custom skill's provider
/// reference lacks the `anthropic` key.
fn append_anthropic_container(
    body: &mut Value,
    options: &CallOptions,
    profile: &RequestProfile,
    betas: &mut BTreeSet<String>,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let Some(container) = anthropic_option(&options.provider_options, profile, "container") else {
        return Ok(());
    };
    let skills = container.get("skills").and_then(|s| s.as_array());
    if let Some(skills_arr) = skills.filter(|a| !a.is_empty()) {
        let mut container_obj = Map::new();
        if let Some(id) = container.get("id")
            && !id.is_null()
        {
            container_obj.insert("id".to_string(), id.clone());
        }
        let mut skills_mapped: Vec<Value> = Vec::new();
        for skill in skills_arr {
            let stype = skill.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let skill_id = if stype == "custom" {
                match skill
                    .get("providerReference")
                    .and_then(|r| r.get(CANONICAL))
                {
                    Some(id) => id.clone(),
                    None => {
                        return Err(AiMuxError::UnsupportedFunctionality(format!(
                            "skill provider reference is missing the '{CANONICAL}' key: {skill}"
                        )));
                    }
                }
            } else {
                skill.get("skillId").cloned().unwrap_or(Value::Null)
            };
            let mut s = Map::new();
            s.insert("type".to_string(), json!(stype));
            s.insert("skill_id".to_string(), skill_id);
            if let Some(v) = skill.get("version")
                && !v.is_null()
            {
                s.insert("version".to_string(), v.clone());
            }
            skills_mapped.push(Value::Object(s));
        }
        container_obj.insert("skills".to_string(), json!(skills_mapped));
        body["container"] = json!(container_obj);
        betas.insert("code-execution-2025-08-25".to_string());
        betas.insert("skills-2025-10-02".to_string());
        betas.insert("files-api-2025-04-14".to_string());

        // Warn when skills are configured without a code execution tool.
        let has_code_exec = options
            .tools
            .as_ref()
            .map(|t| {
                t.iter().any(|tool| match tool {
                    Tool::Provider(pt) => {
                        pt.id == "anthropic.code_execution_20250825"
                            || pt.id == "anthropic.code_execution_20260120"
                    }
                    _ => false,
                })
            })
            .unwrap_or(false);
        if !has_code_exec {
            warnings.push(Warning::Other {
                message: "code execution tool is required when using skills".to_string(),
            });
        }
    } else if let Some(id) = container.get("id")
        && !id.is_null()
    {
        body["container"] = id.clone();
    }
    Ok(())
}

/// Build the Anthropic request body, returning warnings alongside the body.
///
/// Implements the thinking/reasoning pipeline from the TS
/// `anthropic-language-model.ts` `getArgs`: model-capability detection,
/// `rejectsSamplingParameters` stripping, top-level `reasoning` → thinking/
/// effort mapping (provider options take precedence), and thinking-enabled
/// default budget. The single oversized function was split into focused
/// helpers (issue M11); behavior is unchanged.
///
/// # Errors
///
/// Propagates prompt/provider-option conversion errors, e.g.
/// `AiMuxError::InvalidArgument` for an unresolvable file reference or
/// container option.
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    build_request_body_for(model_id, options, stream, &RequestProfile::default())
}

/// [`build_request_body_with_warnings`] for a host of the Messages API other
/// than the first-party endpoint under its canonical name.
///
/// # Errors
///
/// Same as [`build_request_body_with_warnings`].
pub(crate) fn build_request_body_for(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
    profile: &RequestProfile,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut parsed_options = options.clone();
    if let Some(raw) =
        anthropic_options_in(options.provider_options.as_ref(), &profile.options_name)
    {
        let parsed =
            parse_anthropic_option_object("options", &Value::Object(raw)).map_err(|()| {
                AiMuxError::InvalidArgument("invalid anthropic provider options".to_string())
            })?;
        let namespaces = parsed_options
            .provider_options
            .get_or_insert_with(Default::default);
        namespaces.remove(CANONICAL);
        namespaces.insert(
            profile.options_name.clone(),
            parsed.as_object().unwrap().clone(),
        );
    }
    let options = &parsed_options;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut betas: BTreeSet<String> = BTreeSet::new();
    let caps = get_model_capabilities(model_id);

    // Unknown-model max output tokens warning (TS L305-314): for models we do
    // not recognise, note the applied default limit when the caller did not
    // set maxOutputTokens.
    if !caps.is_known_model && options.max_output_tokens.is_none() {
        warnings.push(Warning::Compatibility {
            feature: "maxOutputTokens".to_string(),
            details: Some(format!(
                "The model \"{}\" is unknown. The max output tokens have been limited to {}. Set maxOutputTokens explicitly to override this limit.",
                model_id, caps.max_output_tokens
            )),
        });
    }

    // rejectsSamplingParameters: strip temperature/topK/topP with warnings.
    let (mut temperature, mut top_p, mut top_k) =
        strip_anthropic_sampling_params(options, model_id, &caps, &mut warnings);

    // providerOptions.anthropic.thinking / .effort + top-level `reasoning`.
    let (thinking_config, thinking_effort) =
        resolve_anthropic_thinking(model_id, options, profile, &caps, &mut warnings);
    let (thinking_type, send_thinking, mut thinking_budget, thinking_display) =
        derive_anthropic_thinking(&thinking_config);

    let max_tokens = options.max_output_tokens.unwrap_or(caps.max_output_tokens);

    let send_input_reasoning =
        anthropic_option(&options.provider_options, profile, "sendReasoning")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
    let mut cache_validator = CacheControlValidator::for_options_name(&profile.options_name);
    let conversion = convert_prompt_with_validator(
        &options.prompt,
        send_input_reasoning,
        &ToolNameMapping::new(options.tools.as_deref()),
        &profile.options_name,
        &mut cache_validator,
    )?;
    let system = conversion.system;
    let messages = conversion.messages;
    betas.extend(conversion.betas);
    warnings.extend(conversion.warnings);

    let mut body = json!({
        "model": model_id,
        "messages": messages,
        "max_tokens": max_tokens,
    });
    // `stream` is sent only when streaming, as the AI SDK does.
    if stream {
        body["stream"] = json!(true);
    }

    if let Some(sys) = system {
        body["system"] = json!(sys);
    }

    insert_anthropic_thinking(
        &mut body,
        &thinking_type,
        send_thinking,
        &thinking_budget,
        &thinking_display,
        &thinking_effort,
    );

    if let Some(binding) = thinking_config.as_ref().and_then(|v| v.get("blockBinding")) {
        if body.get("thinking").is_none() {
            body["thinking"] = json!({});
        }
        body["thinking"]["block_binding"] =
            json!({ "prefix_mismatch_behavior": binding.get("prefixMismatchBehavior") });
        betas.insert("thinking-binding-controls-2026-08-01".to_string());
    }

    // Thinking-enabled post-processing (TS L651-696): default budget,
    // sampling-parameter stripping, `max_tokens` adjustment.
    let mut adjusted_max_tokens = apply_anthropic_thinking_post_processing(
        &mut body,
        &thinking_type,
        &mut thinking_budget,
        max_tokens,
        &mut temperature,
        &mut top_k,
        &mut top_p,
        &mut warnings,
    );
    if caps.is_known_model && adjusted_max_tokens > caps.max_output_tokens {
        if options.max_output_tokens.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "maxOutputTokens".to_string(),
                details: Some(format!(
                    "{} (maxOutputTokens + thinkingBudget) is greater than {} {} max output tokens. The max output tokens have been limited to {}.",
                    adjusted_max_tokens, model_id, caps.max_output_tokens, caps.max_output_tokens
                )),
            });
        }
        adjusted_max_tokens = caps.max_output_tokens;
    }
    if adjusted_max_tokens != max_tokens {
        body["max_tokens"] = json!(adjusted_max_tokens);
    }

    if !matches!(
        thinking_type.as_deref(),
        Some("enabled" | "adaptive" | "between_tools")
    ) && (caps.is_known_model || is_legacy_claude(model_id))
        && temperature.is_some()
        && top_p.is_some()
    {
        warnings.push(Warning::Unsupported {
            feature: "topP".to_string(),
            details: Some(
                "topP is not supported when temperature is set. topP is ignored.".to_string(),
            ),
        });
        top_p = None;
    }
    insert_anthropic_sampling(&mut body, temperature, top_p, top_k, options);

    let mut json_tool_schema = None;
    if let Some(ResponseFormat::Json { schema, .. }) = &options.response_format {
        if let Some(schema) = schema {
            let mode = anthropic_option(&options.provider_options, profile, "structuredOutputMode");
            let mut use_native = mode.as_ref().and_then(Value::as_str) == Some("outputFormat")
                || (mode.as_ref().and_then(Value::as_str).unwrap_or("auto") == "auto"
                    && caps.supports_structured_output
                    && profile.supports_native_structured_output);
            if !use_native
                && caps.rejects_forced_tool_use
                && caps.supports_structured_output
                && profile.supports_native_structured_output
            {
                warnings.push(Warning::Unsupported {
                    feature: "providerOptions.anthropic.structuredOutputMode".to_string(),
                    details: Some(format!("structuredOutputMode 'jsonTool' is not supported by {model_id} because it rejects forced tool use. Using 'outputFormat' instead.")),
                });
                use_native = true;
            }
            if !use_native {
                json_tool_schema = Some(schema);
                if anthropic_option(&options.provider_options, profile, "disableParallelToolUse")
                    .and_then(|v| v.as_bool())
                    == Some(false)
                {
                    warnings.push(Warning::Unsupported {
                        feature: "providerOptions.anthropic.disableParallelToolUse".to_string(),
                        details: Some("`disableParallelToolUse: false` is ignored when using the JSON response tool. Parallel tool use is disabled to ensure a single coherent JSON tool call.".to_string()),
                    });
                }
            }
            if use_native {
                if body.get("output_config").is_none() {
                    body["output_config"] = json!({});
                }
                body["output_config"]["format"] = json!({ "type": "json_schema", "schema": super::sanitize_json_schema::sanitize_json_schema(schema) });
            }
        } else {
            warnings.push(Warning::Unsupported {
                feature: "responseFormat".to_string(),
                details: Some(
                    "JSON response format requires a schema. The response format is ignored."
                        .to_string(),
                ),
            });
        }
    }

    // providerOptions.anthropic.metadata.userId -> metadata.user_id.
    if let Some(user_id) = anthropic_option(&options.provider_options, profile, "metadata")
        .and_then(|metadata| metadata.get("userId").cloned())
        .filter(|user_id| !user_id.is_null())
    {
        body["metadata"] = json!({ "user_id": user_id });
    }

    // Tools — provider-defined tools alongside function tools, plus the
    // required beta headers / tool warnings.
    apply_anthropic_tools(
        &mut body,
        options,
        profile,
        stream,
        json_tool_schema,
        &caps,
        &mut betas,
        &mut warnings,
        &mut cache_validator,
    );
    warnings.extend(cache_validator.take_warnings());

    // providerOptions.anthropic.mcpServers → mcp_servers + beta header.
    append_anthropic_mcp_servers(&mut body, options, profile, &mut betas);

    // providerOptions.anthropic.container — programmatic tool calling / skills.
    append_anthropic_container(&mut body, options, profile, &mut betas, &mut warnings)?;

    let provider_options =
        anthropic_options_in(options.provider_options.as_ref(), &profile.options_name);
    if let Some(provider_options) = provider_options {
        if provider_options
            .get("compaction")
            .is_some_and(|v| !v.is_null())
            && provider_options
                .get("contextManagement")
                .is_some_and(|v| !v.is_null())
        {
            return Err(AiMuxError::InvalidArgument("Anthropic provider options `compaction` and `contextManagement` cannot be used together.".to_string()));
        }
        for (source, target) in [
            ("speed", "speed"),
            ("serviceTier", "service_tier"),
            ("inferenceGeo", "inference_geo"),
            ("cacheControl", "cache_control"),
            ("compaction", "compaction"),
        ] {
            if let Some(value) = provider_options.get(source).filter(|v| !v.is_null()) {
                body[target] = value.clone();
            }
        }
        if let Some(value) = provider_options.get("fallbacks").filter(|v| {
            v.as_str() == Some("default") || v.as_array().is_some_and(|a| !a.is_empty())
        }) {
            body["fallbacks"] = value.clone();
            betas.insert(
                if value.as_str() == Some("default") {
                    "server-side-fallback-2026-07-01"
                } else {
                    "server-side-fallback-2026-06-01"
                }
                .to_string(),
            );
        }
        if let Some(value) = provider_options.get("taskBudget").filter(|v| !v.is_null()) {
            if body.get("output_config").is_none() {
                body["output_config"] = json!({});
            }
            body["output_config"]["task_budget"] = value.clone();
            betas.insert("task-budgets-2026-03-13".to_string());
        }
        if body.get("speed").and_then(Value::as_str) == Some("fast") {
            betas.insert("fast-mode-2026-02-01".to_string());
        }
        if body.get("compaction").is_some() {
            betas.insert("compact-2026-09-04".to_string());
        }
        if let Some(edits) = provider_options
            .get("contextManagement")
            .and_then(|v| v.get("edits"))
            .and_then(Value::as_array)
        {
            let mut mapped = Vec::new();
            for edit in edits {
                let strategy = edit.get("type").and_then(Value::as_str).unwrap_or_default();
                let fields: &[(&str, &str)] = match strategy {
                    "clear_tool_uses_20250919" => &[
                        ("trigger", "trigger"),
                        ("keep", "keep"),
                        ("clearAtLeast", "clear_at_least"),
                        ("clearToolInputs", "clear_tool_inputs"),
                        ("excludeTools", "exclude_tools"),
                    ],
                    "clear_thinking_20251015" => &[("keep", "keep")],
                    "compact_20260112" => {
                        betas.insert("compact-2026-01-12".to_string());
                        &[
                            ("trigger", "trigger"),
                            ("pauseAfterCompaction", "pause_after_compaction"),
                            ("instructions", "instructions"),
                        ]
                    }
                    _ => {
                        warnings.push(Warning::Other {
                            message: format!("Unknown context management strategy: {strategy}"),
                        });
                        continue;
                    }
                };
                let mut mapped_edit = json!({ "type": strategy });
                for (source, target) in fields {
                    if let Some(value) = edit.get(*source) {
                        mapped_edit[*target] = value.clone();
                    }
                }
                mapped.push(mapped_edit);
            }
            body["context_management"] = json!({ "edits": mapped });
            betas.insert("context-management-2025-06-27".to_string());
        }
    }
    if let Some(values) = anthropic_option(&options.provider_options, profile, "anthropicBeta")
        .and_then(|value| value.as_array().cloned())
    {
        betas.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
    }
    if thinking_display.as_ref().and_then(Value::as_str) == Some("updates") {
        betas.insert("thinking-display-updates-2026-08-18".to_string());
    }

    Ok(RequestBodyResult {
        uses_json_response_tool: json_tool_schema.is_some(),
        body,
        warnings,
        betas,
    })
}

/// Parse Anthropic stop_reason into `FinishReason`.
#[must_use]
pub fn parse_stop_reason(s: &str) -> FinishReason {
    let unified = match s {
        "end_turn" | "stop_sequence" | "pause_turn" => FinishReasonUnified::Stop,
        "max_tokens" | "model_context_window_exceeded" => FinishReasonUnified::Length,
        "refusal" => FinishReasonUnified::ContentFilter,
        "tool_use" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}
