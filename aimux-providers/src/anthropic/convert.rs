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

use std::collections::{BTreeSet, HashMap};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Warning};
use serde_json::{Map, Value, json};

use crate::anthropic::cache_control::CacheControlValidator;
use crate::anthropic::prepare_tools::{AnthropicTool, prepare_tools_with_provider};

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
pub fn convert_prompt_to_anthropic_full(
    prompt: &LanguageModelPrompt,
    send_reasoning: bool,
) -> AnthropicPromptConversion {
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
                    ) {
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
                    if let Some(block) = convert_part_to_anthropic(
                        p,
                        send_reasoning,
                        &mut betas,
                        &mut warnings,
                        &mut validator,
                        is_last_part,
                        msg.provider_options.as_ref(),
                        "assistant message part",
                        "assistant message",
                    ) {
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
    AnthropicPromptConversion {
        system: system_opt,
        messages,
        betas,
        warnings,
    }
}

/// Convert a prompt into the Anthropic `system` + `messages` shape.
///
/// This is the legacy two-tuple return form used by [`build_request_body`]. It
/// is equivalent to [`convert_prompt_to_anthropic_full`] with
/// `send_reasoning = false`, discarding the betas and warnings. Consecutive
/// messages that map to the same effective Anthropic role (`user`/`tool` →
/// `user`, `assistant` → `assistant`) are merged into a single message, matching
/// the SDK behaviour.
pub fn convert_prompt_to_anthropic(
    prompt: &LanguageModelPrompt,
) -> (Option<Vec<Value>>, Vec<Value>) {
    let result = convert_prompt_to_anthropic_full(prompt, false);
    (result.system, result.messages)
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
) -> Option<Value> {
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

    Some(match part {
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
            let full = resolve_full_media_type(media_type, data);
            let block = route_file_bytes(&full, data, filename.as_deref(), betas);
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
            let full = resolve_full_media_type(media_type, &bytes);
            let block = route_file_base64(&full, data, &bytes, filename.as_deref(), betas);
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(block, cc)
        }

        ContentPart::FileUrl {
            url,
            media_type,
            provider_options,
        } => {
            let block = route_file_url(media_type, url, betas);
            let cc = resolve_cc(validator, provider_options.as_ref());
            apply_cc(block, cc)
        }

        ContentPart::FileReference {
            media_type,
            reference,
            provider_options,
            ..
        } => {
            let file_id = resolve_anthropic_reference(reference);
            betas.insert(BETA_FILES_API.to_string());
            let container_upload = provider_options
                .as_ref()
                .and_then(|o| o.get("anthropic"))
                .and_then(|a| a.get("containerUpload"))
                .and_then(|v| v.as_bool())
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
            return convert_reasoning_part(
                text,
                signature.as_deref(),
                provider_options.as_ref(),
                send_reasoning,
                warnings,
                validator,
            );
        }

        ContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
            provider_options,
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
    })
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
) -> Value {
    use base64::Engine;
    match top_level_media_type(full_media_type) {
        "image" => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            json!({
                "type": "image",
                "source": { "type": "base64", "media_type": full_media_type, "data": b64 }
            })
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
            block
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
            block
        }
        _ => panic!("unsupported functionality: media type: {}", full_media_type),
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
) -> Value {
    match top_level_media_type(full_media_type) {
        "image" => {
            json!({
                "type": "image",
                "source": { "type": "base64", "media_type": full_media_type, "data": b64 }
            })
        }
        "application" if full_media_type == "application/pdf" => {
            betas.insert(BETA_PDFS.to_string());
            let mut block = json!({
                "type": "document",
                "source": { "type": "base64", "media_type": "application/pdf", "data": b64 }
            });
            if let Some(t) = title {
                block["title"] = json!(t);
            }
            block
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
            block
        }
        _ => panic!("unsupported functionality: media type: {}", full_media_type),
    }
}

/// Route a URL file part. No byte-sniffing is possible; routing uses the
/// (possibly top-level-only) media type, matching the TS SDK which only checks
/// `mediaType === 'application/pdf'` / `mediaType === 'text/plain'` and the
/// top-level segment for images.
fn route_file_url(media_type: &str, url: &str, betas: &mut BTreeSet<String>) -> Value {
    match top_level_media_type(media_type) {
        "image" => {
            json!({ "type": "image", "source": { "type": "url", "url": url } })
        }
        "application" if media_type == "application/pdf" => {
            betas.insert(BETA_PDFS.to_string());
            json!({ "type": "document", "source": { "type": "url", "url": url } })
        }
        "text" if media_type == "text/plain" => {
            json!({ "type": "document", "source": { "type": "url", "url": url } })
        }
        _ => panic!("unsupported functionality: media type: {}", media_type),
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
fn resolve_full_media_type(media_type: &str, bytes: &[u8]) -> String {
    if is_full_media_type(media_type) {
        return media_type.to_string();
    }
    let top = top_level_media_type(media_type);
    match detect_media_type(bytes, top) {
        Some(detected) => detected.to_string(),
        None => panic!(
            "unsupported functionality: file of media type \"{}\" must specify subtype since it could not be auto-detected",
            media_type
        ),
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

    if let Some(sig) = signature {
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
fn resolve_anthropic_reference(reference: &Value) -> String {
    if let Some(id) = reference.get("anthropic").and_then(|v| v.as_str()) {
        return id.to_string();
    }
    let providers: Vec<&str> = reference
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    panic!(
        "No provider reference found for provider 'anthropic'. Available providers: {}",
        providers.join(", ")
    );
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

/// Returns `true` when the reasoning effort is a user-chosen level (i.e. not
/// `ProviderDefault` and not absent), mirroring the TS `isCustomReasoning`.
fn is_custom_reasoning(reasoning: Option<ReasoningEffort>) -> bool {
    match reasoning {
        Some(ReasoningEffort::ProviderDefault) | None => false,
        Some(_) => true,
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
                    "reasoning \"{}\" is not supported by this model.",
                    level
                )),
            });
            return None;
        }
    };

    if mapped != level {
        warnings.push(Warning::Compatibility {
            feature: "reasoning".to_string(),
            details: Some(format!(
                "reasoning \"{}\" is not directly supported by this model. mapped to effort \"{}\".",
                level, mapped
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
                    "reasoning \"{}\" is not supported by this model.",
                    reasoning
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
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<Value, AiMuxError> {
    Ok(build_request_body_with_warnings(model_id, options, stream)?.body)
}

/// Build the Anthropic request body, returning warnings alongside the body.
///
/// Implements the thinking/reasoning pipeline from the TS
/// `anthropic-language-model.ts` `getArgs`: model-capability detection,
/// `rejectsSamplingParameters` stripping, top-level `reasoning` → thinking/
/// effort mapping (provider options take precedence), and thinking-enabled
/// default budget, sampling-parameter stripping, and `max_tokens` adjustment.
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let mut betas: BTreeSet<String> = BTreeSet::new();
    let caps = get_model_capabilities(model_id);

    // Unknown-model max output tokens warning (TS L305-314): when the model is
    // not recognised and the caller did not set maxOutputTokens, emit a
    // compatibility warning noting the default limit applied.
    if !caps.is_known_model && options.max_output_tokens.is_none() {
        warnings.push(Warning::Compatibility {
            feature: "maxOutputTokens".to_string(),
            details: Some(format!(
                "The model \"{}\" is unknown. The max output tokens have been limited to {}. Set maxOutputTokens explicitly to override this limit.",
                model_id, caps.max_output_tokens
            )),
        });
    }

    let mut temperature = options.temperature;
    let mut top_p = options.top_p;
    let mut top_k = options.top_k;

    // rejectsSamplingParameters: strip temperature/topK/topP with warnings.
    if caps.rejects_sampling_parameters {
        if temperature.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "temperature".to_string(),
                details: Some(format!(
                    "temperature is not supported by {} and will be ignored",
                    model_id
                )),
            });
            temperature = None;
        }
        if top_k.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: Some(format!(
                    "topK is not supported by {} and will be ignored",
                    model_id
                )),
            });
            top_k = None;
        }
        if top_p.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topP".to_string(),
                details: Some(format!(
                    "topP is not supported by {} and will be ignored",
                    model_id
                )),
            });
            top_p = None;
        }
    }

    // providerOptions.anthropic.thinking / .effort take precedence over the
    // top-level `reasoning`. The top-level mapping only runs when `effort` is
    // not already set by provider options (TS L426-445).
    let mut thinking_config: Option<Value> =
        anthropic_option(&options.provider_options, "thinking");
    let mut effort: Option<String> = anthropic_option(&options.provider_options, "effort")
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    if is_custom_reasoning(options.reasoning) && effort.is_none() {
        let reasoning = options.reasoning.unwrap();
        if let Some(rc) = resolve_anthropic_reasoning_config(
            reasoning,
            caps.supports_adaptive_thinking,
            caps.supports_xhigh_effort,
            caps.max_output_tokens,
            &mut warnings,
        ) {
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
    }

    // Newer models only allow disabling thinking at effort ≤ high; lower
    // xhigh/max to high (TS L451-464).
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

    let thinking_type: Option<String> = thinking_config
        .as_ref()
        .and_then(|t| t.get("type"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());

    let is_thinking =
        thinking_type.as_deref() == Some("enabled") || thinking_type.as_deref() == Some("adaptive");
    // `disabled` must still be forwarded to the API: some models default
    // thinking on, so omitting it would leave thinking enabled.
    let send_thinking = is_thinking || thinking_type.as_deref() == Some("disabled");

    let mut thinking_budget: Option<u32> = if thinking_type.as_deref() == Some("enabled") {
        thinking_config
            .as_ref()
            .and_then(|t| t.get("budgetTokens"))
            .and_then(|v| v.as_u64())
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

    let max_tokens = options.max_output_tokens.unwrap_or(caps.max_output_tokens);

    let (system, messages) = convert_prompt_to_anthropic(&options.prompt);

    let mut body = json!({
        "model": model_id,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": stream,
    });

    if let Some(sys) = system {
        body["system"] = json!(sys);
    }

    if send_thinking && let Some(ref tt) = thinking_type {
        let mut thinking_obj = json!({ "type": tt });
        if let Some(b) = thinking_budget {
            thinking_obj["budget_tokens"] = json!(b);
        }
        if let Some(d) = thinking_display {
            thinking_obj["display"] = d;
        }
        body["thinking"] = thinking_obj;
    }

    if let Some(ref eff) = effort {
        body["output_config"] = json!({ "effort": eff });
    }

    // thinking-enabled post-processing (TS L651-696).
    if is_thinking {
        if thinking_type.as_deref() == Some("enabled") && thinking_budget.is_none() {
            warnings.push(Warning::Compatibility {
                feature: "extended thinking".to_string(),
                details: Some(
                    "thinking budget is required when thinking is enabled. using default budget of 1024 tokens.".to_string(),
                ),
            });
            thinking_budget = Some(1024);
            body["thinking"]["budget_tokens"] = json!(1024);
        }

        if temperature.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "temperature".to_string(),
                details: Some("temperature is not supported when thinking is enabled".to_string()),
            });
            temperature = None;
        }
        if top_k.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: Some("topK is not supported when thinking is enabled".to_string()),
            });
            top_k = None;
        }
        if top_p.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topP".to_string(),
                details: Some("topP is not supported when thinking is enabled".to_string()),
            });
            top_p = None;
        }

        let budget = thinking_budget.unwrap_or(0);
        body["max_tokens"] = json!(max_tokens + budget);
    }

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

    // Tools — delegate to `prepare_tools_with_provider` so that provider-defined
    // tools (web search, code execution, ...) are mapped alongside function
    // tools, and the required beta headers / tool warnings are surfaced.
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

    // providerOptions.anthropic.mcpServers → request body `mcp_servers` +
    // `mcp-client-2025-04-04` beta header.
    if let Some(mcp) = anthropic_option(&options.provider_options, "mcpServers")
        && let Some(arr) = mcp.as_array()
        && !arr.is_empty()
    {
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

    // providerOptions.anthropic.container — programmatic tool calling
    // (string id) or agent skills (object with id + skills).
    if let Some(container) = anthropic_option(&options.provider_options, "container") {
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
                            return Err(AiMuxError::Unsupported(format!(
                                "skill provider reference is missing the 'anthropic' key: {}",
                                skill
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
    }

    // providerOptions.anthropic.contextManagement → request body
    // `context_management` + `context-management-2025-06-27` beta (and
    // `compact-2026-01-12` when a compact edit is present).
    // TODO: implement build_context_management (context management API not yet implemented)
    // if let Some(cm) = anthropic_option(&options.provider_options, "contextManagement") {
    //     if let Some(mapped) = build_context_management(&cm, &mut warnings) {
    //         let has_compact = cm
    //             .get("edits")
    //             .and_then(|e| e.as_array())
    //             .map(|edits| {
    //                 edits.iter().any(|e| {
    //                     e.get("type").and_then(|t| t.as_str()) == Some("compact_20260112")
    //                 })
    //             })
    //             .unwrap_or(false);
    //         betas.insert("context-management-2025-06-27".to_string());
    //         if has_compact {
    //             betas.insert("compact-2026-01-12".to_string());
    //         }
    //         body["context_management"] = mapped;
    //     }
    // }

    Ok(RequestBodyResult {
        body,
        warnings,
        betas,
    })
}

/// Parse Anthropic stop_reason into `FinishReason`.
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
