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

use std::collections::{BTreeSet, HashMap, HashSet};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use serde_json::{Map, Value, json};

use crate::anthropic::cache_control::CacheControlValidator;
use crate::anthropic::prepare_tools::{AnthropicTool, prepare_tools_with_provider};
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
pub fn convert_prompt_to_anthropic_full_with_tools(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
    tool_names: &ToolNameMapping,
) -> Result<AnthropicPromptConversion, AiMuxError> {
    // Ids of tool calls that were executed over MCP. Their results must be sent
    // back as `mcp_tool_result`, not as a provider-tool result block. Upstream
    // scopes this set to one merged assistant block; scanning the whole prompt
    // is a superset of that and is safe because tool call ids are unique.
    let mcp_tool_use_ids = collect_mcp_tool_use_ids(prompt);

    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut betas: BTreeSet<String> = BTreeSet::new();
    let mut warnings: Vec<Warning> = Vec::new();
    let mut validator = CacheControlValidator::new();

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
            messages.push(json!({ "role": role_str, "content": std::mem::take(acc) }));
        }
    }

    for msg in prompt {
        match msg.role {
            Role::System => {
                // A system message interrupts any in-flight user/assistant group.
                flush(&mut messages, &mut last, &mut acc);
                let n = msg.content.len();
                let mut blocks: Vec<Value> = Vec::new();
                for (idx, p) in msg.content.iter().enumerate() {
                    if let ContentPart::Text {
                        text,
                        provider_options,
                    } = p
                    {
                        let is_last = idx + 1 == n;
                        // cache_control: part-level first, then message-level
                        // for the last text part (system content is a string in
                        // the TS model, so message-level is the primary path).
                        let cache_control = match validator.get_cache_control(
                            provider_options.as_ref(),
                            "system message part",
                            true,
                        ) {
                            Some(v) => Some(v),
                            None => {
                                if is_last {
                                    validator.get_cache_control(
                                        msg.provider_options.as_ref(),
                                        "system message",
                                        true,
                                    )
                                } else {
                                    None
                                }
                            }
                        };
                        let mut block = json!({ "type": "text", "text": text });
                        if let Some(cc) = cache_control {
                            block["cache_control"] = cc;
                        }
                        blocks.push(block);
                    }
                    // Non-text parts on a system message are ignored.
                }

                if !seen_non_system {
                    // Initial system block(s) become the top-level system prompt.
                    system.extend(blocks);
                } else {
                    // A mid-conversation system message is emitted inline.
                    messages.push(json!({ "role": "system", "content": blocks }));
                    betas.insert(BETA_MID_CONVERSATION_SYSTEM.to_string());
                }
            }
            Role::User | Role::Tool => {
                seen_non_system = true;
                if last != Some(Eff::User) {
                    flush(&mut messages, &mut last, &mut acc);
                }
                let (part_ctx, msg_ctx) = match msg.role {
                    Role::Tool => ("tool result part", "tool result message"),
                    _ => ("user message part", "user message"),
                };
                let n = msg.content.len();
                for (idx, p) in msg.content.iter().enumerate() {
                    let is_last_part = idx + 1 == n;
                    if let Some(block) = convert_part_to_anthropic(
                        p,
                        send_reasoning,
                        &mut betas,
                        &mut warnings,
                        &mut validator,
                        is_last_part,
                        msg.provider_options.as_ref(),
                        part_ctx,
                        msg_ctx,
                    )? {
                        acc.push(block);
                    }
                }
                last = Some(Eff::User);
            }
            Role::Assistant => {
                seen_non_system = true;
                if last != Some(Eff::Assistant) {
                    flush(&mut messages, &mut last, &mut acc);
                }
                let n = msg.content.len();
                for (idx, p) in msg.content.iter().enumerate() {
                    let is_last_part = idx + 1 == n;
                    // Assistant-role tool results are provider-executed results;
                    // Anthropic only accepts a bare `tool_result` in a *user*
                    // message, so they take a dedicated path.
                    let block = if let ContentPart::ToolResult { .. } = p {
                        convert_assistant_tool_result(
                            p,
                            tool_names,
                            &mcp_tool_use_ids,
                            &mut warnings,
                            &mut validator,
                            is_last_part,
                            msg.provider_options.as_ref(),
                        )
                    } else {
                        convert_part_to_anthropic(
                            p,
                            send_reasoning,
                            &mut betas,
                            &mut warnings,
                            &mut validator,
                            is_last_part,
                            msg.provider_options.as_ref(),
                            "assistant message part",
                            "assistant message",
                        )?
                    };
                    if let Some(block) = block {
                        acc.push(block);
                    }
                }
                last = Some(Eff::Assistant);
            }
        }
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

/// Return the top-level media type (the segment before the first `/`).
fn top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}

/// Convert a single content part into an Anthropic content block.
///
/// Returns `None` when the part should be omitted entirely (e.g. a reasoning
/// part that is dropped). `betas` and `warnings` are appended to as side
/// effects.
///
/// `cache_control` is resolved from the part's `provider_options.anthropic
/// .cacheControl`, falling back to the message-level `provider_options` for the
/// last part of a message (mirroring the TS SDK). Thinking blocks reject
/// cache_control (validated with a warning).
#[allow(clippy::too_many_arguments)]
fn convert_part_to_anthropic(
    part: &ContentPart,
    send_reasoning: bool,
    betas: &mut BTreeSet<String>,
    warnings: &mut Vec<Warning>,
    validator: &mut CacheControlValidator,
    is_last_part: bool,
    message_provider_options: Option<&Value>,
    part_context_type: &str,
    message_context_type: &str,
) -> Result<Option<Value>, AiMuxError> {
    // Resolve cache_control = part-level ?? (is_last_part ? message-level).
    let resolve_cc =
        |validator: &mut CacheControlValidator, part_opts: Option<&Value>| match validator
            .get_cache_control(part_opts, part_context_type, true)
        {
            Some(v) => Some(v),
            None => {
                if is_last_part {
                    validator.get_cache_control(
                        message_provider_options,
                        message_context_type,
                        true,
                    )
                } else {
                    None
                }
            }
        };

    let apply_cc = |block: Value, cc: Option<Value>| -> Value {
        match cc {
            Some(c) => {
                let mut b = block;
                b["cache_control"] = c;
                b
            }
            None => block,
        }
    };

    Ok(Some(match part {
        ContentPart::Text {
            text,
            provider_options,
        } => {
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(json!({ "type": "text", "text": text }), cc)
        }

        ContentPart::Image {
            image,
            media_type,
            provider_options,
        } => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(
                json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": b64,
                    }
                }),
                cc,
            )
        }

        ContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            let full = resolve_full_media_type(media_type, data)?;
            let block = route_file_bytes(&full, data, filename.as_deref(), betas)?;
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(block, cc)
        }

        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap_or_default();
            let full = resolve_full_media_type(media_type, &bytes)?;
            let block = route_file_base64(&full, data, &bytes, filename.as_deref(), betas)?;
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(block, cc)
        }

        ContentPart::FileUrl {
            url,
            media_type,
            provider_options,
        } => {
            let block = route_file_url(media_type, url, betas)?;
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(block, cc)
        }

        ContentPart::FileReference {
            media_type,
            reference,
            provider_options,
            ..
        } => {
            let file_id = resolve_anthropic_reference(reference)?;
            betas.insert(BETA_FILES_API.to_string());
            let container_upload = provider_options
                .as_ref()
                .and_then(|o| o.get("anthropic"))
                .and_then(|a| a.get("containerUpload"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let block = if container_upload {
                json!({ "type": "container_upload", "file_id": file_id })
            } else if top_level_media_type(media_type) == "image" {
                json!({ "type": "image", "source": { "type": "file", "file_id": file_id } })
            } else {
                json!({ "type": "document", "source": { "type": "file", "file_id": file_id } })
            };
            // container_upload blocks do not carry cache_control in the TS SDK.
            let cc = if container_upload {
                None
            } else {
                resolve_cc(validator, provider_options.as_ref())
            };
            apply_cc(block, cc)
        }

        ContentPart::Reasoning {
            text,
            signature,
            provider_options,
        } => {
            return Ok(convert_reasoning_part(
                text,
                signature.as_deref(),
                provider_options.as_ref(),
                send_reasoning,
                warnings,
                validator,
            ));
        }

        ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_options,
            ..
        } => {
            // Anthropic requires `input` to be a JSON object. The SDK wraps any
            // non-object (e.g. malformed JSON the model produced) in
            // `{ "rawInvalidInput": <input> }`.
            let input_val = if input.is_object() {
                input.clone()
            } else {
                json!({ "rawInvalidInput": input })
            };
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(
                json!({
                    "type": "tool_use",
                    "id": tool_call_id,
                    "name": tool_name,
                    "input": input_val,
                }),
                cc,
            )
        }

        ContentPart::ToolResult {
            tool_call_id,
            result,
            provider_options,
            ..
        } => {
            let (content, is_error) = resolve_tool_result_output(result);
            let mut block = json!({
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": content,
            });
            if is_error {
                block["is_error"] = json!(true);
            }
            // cache_control: part ?? output ?? (is_last_part ? message).
            let cc = match validator.get_cache_control(
                provider_options.as_ref(),
                part_context_type,
                true,
            ) {
                Some(v) => Some(v),
                None => match extract_tool_result_output_provider_options(result) {
                    Some(out_opts) => {
                        validator.get_cache_control(Some(out_opts), "tool result output", true)
                    }
                    None => {
                        if is_last_part {
                            validator.get_cache_control(
                                message_provider_options,
                                message_context_type,
                                true,
                            )
                        } else {
                            None
                        }
                    }
                },
            };
            apply_cc(block, cc)
        }
    }))
}

// ── assistant-role tool results (provider-executed) ─────────────────────────

/// Collect the tool call ids that were executed over MCP.
///
/// Their results have to go back as `mcp_tool_result`, so they are indexed
/// before any message is converted. The marker is the one the response side
/// writes on an `mcp_tool_use` block (`stream.rs`):
/// `providerOptions.anthropic.type == "mcp-tool-use"`.
fn collect_mcp_tool_use_ids(prompt: &LanguageModelPrompt) -> HashSet<&str> {
    let mut ids = HashSet::new();
    for msg in prompt {
        if msg.role != Role::Assistant {
            continue;
        }
        for part in &msg.content {
            if let ContentPart::ToolCall {
                tool_call_id,
                provider_options: Some(opts),
                ..
            } = part
                && opts
                    .get("anthropic")
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
    value
        .get("errorCode")
        .or_else(|| value.get("error_code"))
        .and_then(|c| c.as_str())
        .unwrap_or(fallback)
        .to_string()
}

/// Convert an assistant-role `ContentPart::ToolResult` into the matching
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
fn convert_assistant_tool_result(
    part: &ContentPart,
    tool_names: &ToolNameMapping,
    mcp_tool_use_ids: &HashSet<&str>,
    warnings: &mut Vec<Warning>,
    validator: &mut CacheControlValidator,
    is_last_part: bool,
    message_provider_options: Option<&Value>,
) -> Option<Value> {
    let ContentPart::ToolResult {
        tool_call_id,
        tool_name,
        result,
        is_error,
        provider_options,
        ..
    } = part
    else {
        return None;
    };

    // cache_control: part ?? output ?? (is_last_part ? message) — the same
    // resolution order the bare `tool_result` path uses.
    let cache_control = match validator.get_cache_control(
        provider_options.as_ref(),
        "assistant message part",
        true,
    ) {
        Some(v) => Some(v),
        None => match extract_tool_result_output_provider_options(result) {
            Some(out_opts) => {
                validator.get_cache_control(Some(out_opts), "tool result output", true)
            }
            None => {
                if is_last_part {
                    validator.get_cache_control(message_provider_options, "assistant message", true)
                } else {
                    None
                }
            }
        },
    };

    // The payload is carried bare, with the error flag alongside it. Upstream
    // reads both off a `{ type, value }` envelope instead; reconstructing that
    // envelope here would mean guessing, and a legitimate payload that happens
    // to look like one (`{"type":"error","value":...}`) would be unwrapped and
    // have its `is_error` overwritten. The envelope belongs on the type.
    let value = result;
    let is_error = is_error.unwrap_or(false);
    let tool_name = tool_name.as_deref().unwrap_or_default();
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
                assistant_web_search_content(value),
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
fn extract_tool_result_output_provider_options(output: &Value) -> Option<&Value> {
    if let Some(opts) = output.get("providerOptions") {
        return Some(opts);
    }
    let output_type = output.get("type").and_then(|t| t.as_str());
    if output_type == Some("content")
        && let Some(value) = output.get("value").and_then(|v| v.as_array())
    {
        for part in value {
            if let Some(opts) = part.get("providerOptions") {
                return Some(opts);
            }
        }
    }
    None
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
    match top_level_media_type(full_media_type) {
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
    match top_level_media_type(full_media_type) {
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
    match top_level_media_type(media_type) {
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

/// Returns `true` only when the media type has a non-empty, non-wildcard
/// subtype (i.e. `type/subtype` where `subtype` is not `*`).
fn is_full_media_type(media_type: &str) -> bool {
    match media_type.find('/') {
        Some(i) => {
            let subtype = &media_type[i + 1..];
            !subtype.is_empty() && subtype != "*"
        }
        None => false,
    }
}

/// Sniff the concrete media type from inline bytes for the given top-level
/// segment, mirroring the TS `detectMediaType` signature tables.
fn detect_media_type(bytes: &[u8], top_level: &str) -> Option<&'static str> {
    match top_level {
        "image" => detect_image_media_type(bytes),
        "application" => detect_document_media_type(bytes),
        _ => None,
    }
}

fn detect_image_media_type(bytes: &[u8]) -> Option<&'static str> {
    const SIGS: &[(&[u8], &str)] = &[
        (&[0x47, 0x49, 0x46], "image/gif"),
        (&[0x89, 0x50, 0x4e, 0x47], "image/png"),
        (&[0xff, 0xd8], "image/jpeg"),
        (&[0x42, 0x4d], "image/bmp"),
        (&[0x49, 0x49, 0x2a, 0x00], "image/tiff"),
        (&[0x4d, 0x4d, 0x00, 0x2a], "image/tiff"),
    ];
    for (prefix, mt) in SIGS {
        if bytes.starts_with(prefix) {
            return Some(mt);
        }
    }
    // WEBP: "RIFF" .... "WEBP"
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn detect_document_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        return Some("application/pdf");
    }
    None
}

/// Resolve a file part's media type to a full `type/subtype` form, mirroring
/// the TS `resolveFullMediaType`. When the media type is already a full type it
/// is returned as-is; otherwise the subtype is sniffed from the inline bytes.
fn resolve_full_media_type(media_type: &str, bytes: &[u8]) -> Result<String, AiMuxError> {
    if is_full_media_type(media_type) {
        return Ok(media_type.to_string());
    }
    let top = top_level_media_type(media_type);
    match detect_media_type(bytes, top) {
        Some(detected) => Ok(detected.to_string()),
        None => Err(AiMuxError::UnsupportedFunctionality(format!(
            "file of media type \"{media_type}\" must specify subtype since it could not be auto-detected"
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
    signature: Option<&str>,
    provider_options: Option<&Value>,
    send_reasoning: bool,
    warnings: &mut Vec<Warning>,
    validator: &mut CacheControlValidator,
) -> Option<Value> {
    if !send_reasoning {
        warnings.push(Warning::Other {
            message: "sending reasoning content is disabled for this model".to_string(),
        });
        return None;
    }

    // `redactedData` is read from `providerOptions.anthropic.redactedData`
    // (mirroring the TS `anthropicReasoningMetadataSchema`).
    let redacted_data = provider_options
        .and_then(|o| o.get("anthropic"))
        .and_then(|a| a.get("redactedData"))
        .and_then(|v| v.as_str());

    // #6: Fall back to `providerOptions.anthropic.signature` when the
    // `signature` field is None (upstream convert-to-anthropic-prompt.ts:669-690).
    let effective_signature = signature.or_else(|| {
        provider_options
            .and_then(|o| o.get("anthropic"))
            .and_then(|a| a.get("signature"))
            .and_then(|v| v.as_str())
    });

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
/// TS `resolveProviderReference`. Panics when no `anthropic` key is present,
/// matching the TS `UnsupportedFunctionalityError`.
fn resolve_anthropic_reference(reference: &Value) -> Result<String, AiMuxError> {
    if let Some(id) = reference.get("anthropic").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let providers: Vec<&str> = reference
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    Err(AiMuxError::InvalidArgument(format!(
        "No provider reference found for provider 'anthropic'. Available providers: {}",
        providers.join(", ")
    )))
}

/// Resolve a `ContentPart::ToolResult` `output` value into the Anthropic
/// `tool_result.content` (and whether it is an error), mirroring the TS SDK.
///
/// The V4 `tool-result` `output` is a discriminated `{ type, value }` object:
/// - `json` → `JSON.stringify(value)` (a string),
/// - `text` → `value` as-is,
/// - `error` → `value` as-is with `is_error: true`,
/// - `content` → `value` as-is (an array of content blocks),
/// - anything else → the `output` is passed through unchanged.
fn resolve_tool_result_output(output: &Value) -> (Value, bool) {
    if let (Some(t), Some(v)) = (
        output.get("type").and_then(|x| x.as_str()),
        output.get("value"),
    ) {
        match t {
            "json" => (Value::String(v.to_string()), false),
            "text" => (v.clone(), false),
            "error" => (v.clone(), true),
            "content" => (v.clone(), false),
            _ => (output.clone(), false),
        }
    } else {
        (output.clone(), false)
    }
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
            supports_structured_output,
            is_known_model,
        }
    }

    if model_id.contains("claude-opus-5") {
        caps(128000, true, true, true, true, true, true)
    } else if model_id.contains("claude-opus-4-8")
        || model_id.contains("claude-opus-4-7")
        || model_id.contains("claude-fable-5")
        || model_id.contains("claude-sonnet-5")
    {
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
            thinking: json!({ "type": "adaptive" }),
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
    pub body: Value,
    pub warnings: Vec<Warning>,
    /// Beta headers (e.g. `code-execution-2025-08-25`, `mcp-client-2025-04-04`)
    /// that must be sent on the request via the `anthropic-beta` header.
    pub betas: BTreeSet<String>,
}

/// Read a value from `provider_options["anthropic"][key]`.
fn anthropic_option(options: &Option<HashMap<String, Value>>, key: &str) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| m.get("anthropic"))
        .and_then(|o| o.get(key))
        .cloned()
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
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) -> (Option<Value>, Option<String>) {
    let mut thinking_config: Option<Value> =
        anthropic_option(&options.provider_options, "thinking");
    let mut effort: Option<String> = anthropic_option(&options.provider_options, "effort")
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

    let is_thinking =
        thinking_type.as_deref() == Some("enabled") || thinking_type.as_deref() == Some("adaptive");
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
    let is_thinking =
        thinking_type.as_deref() == Some("enabled") || thinking_type.as_deref() == Some("adaptive");
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

    max_tokens + thinking_budget.unwrap_or(0)
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
fn apply_anthropic_tools(
    body: &mut Value,
    options: &CallOptions,
    stream: bool,
    caps: &ModelCapabilities,
    betas: &mut BTreeSet<String>,
    warnings: &mut Vec<Warning>,
) {
    let disable_parallel_tool_use =
        anthropic_option(&options.provider_options, "disableParallelToolUse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let default_eager_input_streaming = stream
        && anthropic_option(&options.provider_options, "toolStreaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

    let anthropic_tools: Vec<AnthropicTool> = match &options.tools {
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

    let prepared = prepare_tools_with_provider(
        if options.tools.is_some() {
            Some(&anthropic_tools)
        } else {
            None
        },
        Some(&options.tool_choice),
        disable_parallel_tool_use,
        caps.supports_structured_output,
        caps.supports_structured_output,
        default_eager_input_streaming,
    );

    warnings.extend(prepared.tool_warnings);
    betas.extend(prepared.betas);

    if let Some(tool_defs) = prepared.tools {
        // Strip null-valued fields from tool definitions so the serialized
        // request body matches the TS behaviour (JSON.stringify drops
        // `undefined` args on provider-defined tools such as web_search).
        let mut defs = tool_defs;
        for def in defs.iter_mut() {
            strip_null_fields(def);
        }
        body["tools"] = json!(defs);
    }
    if let Some(tool_choice) = prepared.tool_choice {
        body["tool_choice"] = tool_choice;
    }
}

/// `providerOptions.anthropic.mcpServers` → `mcp_servers` + the
/// `mcp-client-2025-04-04` beta header.
fn append_anthropic_mcp_servers(
    body: &mut Value,
    options: &CallOptions,
    betas: &mut BTreeSet<String>,
) {
    let Some(mcp) = anthropic_option(&options.provider_options, "mcpServers") else {
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
            if let Some(tc) = server.get("toolConfiguration") {
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
    betas: &mut BTreeSet<String>,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let Some(container) = anthropic_option(&options.provider_options, "container") else {
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
                    .and_then(|r| r.get("anthropic"))
                {
                    Some(id) => id.clone(),
                    None => {
                        return Err(AiMuxError::UnsupportedFunctionality(format!(
                            "skill provider reference is missing the 'anthropic' key: {skill}"
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
        resolve_anthropic_thinking(model_id, options, &caps, &mut warnings);
    let (thinking_type, send_thinking, mut thinking_budget, thinking_display) =
        derive_anthropic_thinking(&thinking_config);

    let max_tokens = options.max_output_tokens.unwrap_or(caps.max_output_tokens);

    // Extended-thinking multi-turn: when thinking is enabled/adaptive, the
    // prior assistant turns' reasoning parts (carrying their signatures)
    // must be echoed back as thinking blocks — Anthropic rejects tool-use
    // continuations that omit them, and the reasoning context is lost
    // otherwise. With thinking disabled the blocks would be rejected, so
    // the parts are omitted with a warning instead.
    let send_input_reasoning =
        matches!(thinking_type.as_deref(), Some("enabled") | Some("adaptive"));
    let conversion = convert_prompt_to_anthropic_full_with_tools(
        &options.prompt,
        send_input_reasoning,
        &ToolNameMapping::new(options.tools.as_deref()),
    )?;
    let system = conversion.system;
    let messages = conversion.messages;
    betas.extend(conversion.betas);
    warnings.extend(conversion.warnings);

    let mut body = json!({
        "model": model_id,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": stream,
    });

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

    // Thinking-enabled post-processing (TS L651-696): default budget,
    // sampling-parameter stripping, `max_tokens` adjustment.
    let adjusted_max_tokens = apply_anthropic_thinking_post_processing(
        &mut body,
        &thinking_type,
        &mut thinking_budget,
        max_tokens,
        &mut temperature,
        &mut top_k,
        &mut top_p,
        &mut warnings,
    );
    if adjusted_max_tokens != max_tokens {
        body["max_tokens"] = json!(adjusted_max_tokens);
    }

    insert_anthropic_sampling(&mut body, temperature, top_p, top_k, options);

    // Tools — provider-defined tools alongside function tools, plus the
    // required beta headers / tool warnings.
    apply_anthropic_tools(&mut body, options, stream, &caps, &mut betas, &mut warnings);

    // providerOptions.anthropic.mcpServers → mcp_servers + beta header.
    append_anthropic_mcp_servers(&mut body, options, &mut betas);

    // providerOptions.anthropic.container — programmatic tool calling / skills.
    append_anthropic_container(&mut body, options, &mut betas, &mut warnings)?;

    // providerOptions.anthropic.contextManagement → context_management is not
    // yet implemented (build_context_management pending).

    // Per-call request body overrides (RFC-0017): deep-merge user-supplied
    // JSON into the built body. `null` values delete the corresponding key.
    if let Some(ref overrides) = options.body_overrides {
        crate::openai::convert::deep_merge_json(&mut body, overrides);
    }

    Ok(RequestBodyResult {
        body,
        warnings,
        betas,
    })
}

/// Parse Anthropic stop_reason into `FinishReason`.
#[must_use]
pub fn parse_stop_reason(s: &str) -> FinishReason {
    let unified = match s {
        "end_turn" => FinishReasonUnified::Stop,
        "max_tokens" => FinishReasonUnified::Length,
        "tool_use" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_with_anthropic_thinking(thinking: serde_json::Value) -> CallOptions {
        let mut provider = std::collections::HashMap::new();
        provider.insert("anthropic".to_string(), thinking);
        CallOptions {
            provider_options: Some(provider),
            ..CallOptions::new(LanguageModelPrompt::default())
        }
    }

    /// Regression (audit finding on the M11 split): when `thinking.enabled` is
    /// set without an explicit `budgetTokens`, the body must carry both the
    /// default `budget_tokens: 1024` **and** the adjusted `max_tokens`
    /// (base + 1024), exactly like the pre-split implementation.
    #[test]
    fn thinking_enabled_without_budget_writes_default_budget() {
        let opts = opts_with_anthropic_thinking(json!({
            "thinking": { "type": "enabled" }
        }));
        let req = build_request_body_with_warnings("claude-sonnet-4-5", &opts, false).unwrap();
        assert_eq!(req.body["thinking"]["type"], json!("enabled"));
        assert_eq!(req.body["thinking"]["budget_tokens"], json!(1024));
        // claude-sonnet-4-5 caps: 64000 tokens, +1024 thinking budget.
        assert_eq!(req.body["max_tokens"], json!(65024));
    }

    /// Explicit `budgetTokens` must be preserved (no default override).
    #[test]
    fn thinking_enabled_with_explicit_budget_keeps_it() {
        let opts = opts_with_anthropic_thinking(json!({
            "thinking": { "type": "enabled", "budgetTokens": 4096 }
        }));
        let req = build_request_body_with_warnings("claude-sonnet-4-5", &opts, false).unwrap();
        assert_eq!(req.body["thinking"]["budget_tokens"], json!(4096));
        assert_eq!(req.body["max_tokens"], json!(64000 + 4096));
    }
}
