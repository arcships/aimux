//! Conversion between `LanguageModelPrompt` and Google Gemini API format.
//!
//! Mirrors the TS SDK's `convert-to-google-messages.ts` and the request-body
//! construction inside `google-language-model.ts`'s `getArgs`. The Gemini
//! request shape lifts system content out of the message list:
//!
//! - System messages are lifted out of `contents` into a top-level
//!   `systemInstruction` field (a `{ parts: [{ text }] }` object).
//! - Assistant messages become `role: "model"`.
//! - Tool results become `functionResponse` parts inside a `role: "user"`
//!   message (Gemini has no `tool` role).
//! - Tool calls in assistant messages become `functionCall` parts, except
//!   provider-executed server transcripts which retain their native wire
//!   representation.
//!
//! We model the variable-shape content parts as `serde_json::Value` to keep
//! the surface area small — the TS SDK uses a tagged union, but the only
//! fields we actually read back are `text`, function tool parts, and native
//! provider-executed tool parts.

use super::options::{GOOGLE, Namespace};
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::result::{GenerateContent, Source};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, Warning};
use base64::Engine;
use serde_json::{Map, Value, json};

/// Resolve the caller-facing name for Google's code execution provider tool.
/// Gemini always uses `code_execution` on the response wire, while callers
/// may rename the provider tool for a particular request.
pub(crate) fn code_execution_tool_name(tools: Option<&[Tool]>) -> String {
    tools
        .unwrap_or_default()
        .iter()
        .find_map(|tool| match tool {
            Tool::Provider(provider) if provider.id == "google.code_execution" => {
                Some(provider.name.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| "code_execution".to_string())
}

// ── Public conversion result ─────────────────────────────────────────────────

/// The result of converting a `LanguageModelPrompt` into Google's
/// `contents` + `systemInstruction` shape.
#[derive(Debug, Clone, Default)]
pub struct GooglePrompt {
    /// Top-level `systemInstruction` object, or `None` when there are no
    /// system messages (or for Gemma models, which don't accept it).
    pub system_instruction: Option<Value>,
    /// The `contents` array — one entry per non-system message (tool messages
    /// are folded into the preceding user turn as `functionResponse` parts).
    pub contents: Vec<Value>,
}

// ── convertToGoogleMessages ──────────────────────────────────────────────────

/// Convert a provider-facing prompt into Google's `{ systemInstruction, contents }`.
///
/// Mirrors `convertToGoogleMessages` in the TS SDK with the simplifications
/// appropriate to the Rust data model:
/// - No Gemma special-casing (we don't know the model id here; callers that
///   need it can post-process).
/// - Provider-executed calls and results use their provider metadata to replay
///   native `toolCall` / `toolResponse` or `executableCode` /
///   `codeExecutionResult` parts.
/// - Tool-result JSON stays structured, except values containing `$ref`, which
///   are stringified to avoid Google's multimodal reference handling.
///
/// Thought signatures from provider options are echoed as a
/// `thoughtSignature` sibling of the `functionCall` part.
#[must_use]
pub fn convert_to_google_messages(prompt: &LanguageModelPrompt) -> GooglePrompt {
    convert_to_google_messages_for_namespace(prompt, Namespace::Google, true)
}

fn convert_to_google_messages_for_namespace(
    prompt: &LanguageModelPrompt,
    namespace: Namespace,
    supports_function_response_parts: bool,
) -> GooglePrompt {
    let mut system_parts: Vec<Value> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut system_messages_allowed = true;

    for msg in prompt {
        match msg {
            LanguageModelMessage::System { content, .. } => {
                if !system_messages_allowed {
                    // The TS SDK throws `UnsupportedFunctionalityError` here:
                    // system messages are only valid at the start of the
                    // conversation. This function returns `GooglePrompt` (not
                    // `Result`), so we cannot propagate an error. Dropping the
                    // late system message is safer than folding it into
                    // `systemInstruction` (which would make Gemini treat a
                    // mid-conversation instruction as a global rule).
                    // TODO: change the signature to `Result` to match TS semantics.
                    continue;
                }
                system_parts.push(json!({ "text": content }));
            }
            LanguageModelMessage::User { content, .. } => {
                system_messages_allowed = false;
                let parts = convert_user_parts(content);
                contents.push(json!({ "role": "user", "parts": parts }));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                system_messages_allowed = false;
                let parts = convert_assistant_parts(content, namespace);
                contents.push(json!({ "role": "model", "parts": parts }));
            }
            LanguageModelMessage::Tool { content, .. } => {
                system_messages_allowed = false;
                // Gemini folds tool results into a `user`-role message as
                // `functionResponse` parts. We emit them as their own user
                // turn (matching how the TS SDK pushes a new `{role:'user'}`
                // entry per tool-role message).
                let mut ordinary = Vec::new();
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        output,
                        provider_options,
                        ..
                    }) = part
                    else {
                        continue;
                    };
                    if let Some(opts) = namespace.read_part(provider_options.as_ref())
                        && let (Some(id), Some(kind)) =
                            (opts.get("serverToolCallId"), opts.get("serverToolType"))
                        && let Some(last) = contents.last_mut()
                        && last["role"] == "model"
                    {
                        let mut response = json!({ "toolResponse": { "toolType": kind, "response": match output { ToolResultOutput::Json { value, .. } => value.clone(), _ => json!({}) }, "id": id } });
                        if let Some(signature) = opts.get("thoughtSignature") {
                            response["thoughtSignature"] = signature.clone();
                        }
                        last["parts"].as_array_mut().unwrap().push(response);
                    } else {
                        ordinary.push(part.clone());
                    }
                }
                let parts =
                    convert_tool_parts(&ordinary, namespace, supports_function_response_parts);
                contents.push(json!({ "role": "user", "parts": parts }));
            }
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(json!({ "parts": system_parts }))
    };

    GooglePrompt {
        system_instruction,
        contents,
    }
}

/// Convert user-role content parts into Google parts.
fn convert_user_parts(content: &[UserPart]) -> Vec<Value> {
    content
        .iter()
        .map(|part| match part {
            UserPart::Text(TextPart { text, .. }) => json!({ "text": text }),
            UserPart::File(file) => convert_file_part(file),
        })
        .collect()
}

fn convert_file_part(file: &FilePart) -> Value {
    let media_type = if matches!(file.data, FileData::Text { .. }) {
        if aimux_provider_utils::is_full_media_type(&file.media_type) {
            file.media_type.clone()
        } else {
            "text/plain".into()
        }
    } else {
        aimux_provider_utils::resolve_full_media_type(file)
            .unwrap_or_else(|_| file.media_type.clone())
    };
    let data = match &file.data {
        FileData::Data {
            data: FileBytes::Binary(bytes),
        } => base64::engine::general_purpose::STANDARD.encode(bytes),
        FileData::Data {
            data: FileBytes::Base64(data),
        } => data.clone(),
        FileData::Text { text } => {
            base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
        }
        FileData::Url { url, original_url } => {
            let uri = if url.starts_with("gs:") {
                original_url.as_deref().unwrap_or(url)
            } else {
                url
            };
            return json!({ "fileData": { "mimeType": media_type, "fileUri": uri } });
        }
        FileData::Reference { reference } => {
            return json!({ "fileData": { "mimeType": media_type, "fileUri": reference.get(GOOGLE) } });
        }
    };
    json!({ "inlineData": { "mimeType": media_type, "data": data } })
}

/// Convert assistant-role content parts into Google `model`-role parts.
///
/// - `Text` → `{ text }` (skipped when empty, matching the TS SDK).
/// - `ToolCall` → `{ functionCall: { id?, name, args } }`.
fn convert_assistant_parts(content: &[AssistantPart], namespace: Namespace) -> Vec<Value> {
    let mut parts = Vec::new();
    for part in content {
        match part {
            AssistantPart::Text(TextPart {
                text,
                provider_options,
            }) => {
                if !text.is_empty() {
                    let mut p = json!({ "text": text });
                    // Echo thoughtSignature from provider_options if present
                    // (upstream convert-to-google-messages.ts:355-377).
                    if let Some(sig) = namespace
                        .read_part(provider_options.as_ref())
                        .and_then(|g| g.get("thoughtSignature"))
                        .and_then(|v| v.as_str())
                    {
                        p["thoughtSignature"] = json!(sig);
                    }
                    parts.push(p);
                }
            }
            AssistantPart::Reasoning(ReasoningPart {
                text,
                provider_options,
            }) => {
                if !text.is_empty() {
                    let mut p = json!({ "text": text, "thought": true });
                    if let Some(sig) = namespace
                        .read_part(provider_options.as_ref())
                        .and_then(|g| g.get("thoughtSignature"))
                        .and_then(|v| v.as_str())
                    {
                        p["thoughtSignature"] = json!(sig);
                    }
                    parts.push(p);
                }
            }
            AssistantPart::ToolCall(ToolCallPart {
                tool_call_id,
                tool_name,
                input,
                provider_options,
                provider_executed,
            }) => {
                if *provider_executed == Some(true) && tool_name == "code_execution" {
                    let input = if let Value::String(input) = input {
                        serde_json::from_str(input).unwrap_or(Value::Null)
                    } else {
                        input.clone()
                    };
                    parts.push(json!({ "executableCode": input }));
                    continue;
                }
                let google_options = namespace.read_part(provider_options.as_ref());
                let server_tool_call_id = google_options
                    .and_then(|options| options.get("serverToolCallId"))
                    .and_then(|value| value.as_str());
                let server_tool_type = google_options
                    .and_then(|options| options.get("serverToolType"))
                    .and_then(|value| value.as_str());
                let signature = google_options
                    .and_then(|options| options.get("thoughtSignature"))
                    .and_then(Value::as_str);

                let mut part_value = if let (Some(server_id), Some(server_type)) =
                    (server_tool_call_id, server_tool_type)
                {
                    let args = match input {
                        Value::String(raw) => {
                            serde_json::from_str(raw).unwrap_or_else(|_| input.clone())
                        }
                        _ => input.clone(),
                    };
                    if server_type == "code_execution" {
                        json!({ "executableCode": args })
                    } else {
                        json!({
                            "toolCall": {
                                "toolType": server_type,
                                "args": args,
                                "id": server_id,
                            }
                        })
                    }
                } else {
                    let mut function_call = Map::new();
                    if namespace == Namespace::Google && !tool_call_id.is_empty() {
                        function_call.insert("id".to_string(), json!(tool_call_id));
                    }
                    function_call.insert("name".to_string(), json!(tool_name));
                    function_call.insert("args".to_string(), input.clone());
                    json!({ "functionCall": function_call })
                };
                if let Some(signature) = signature {
                    part_value["thoughtSignature"] = json!(signature);
                }
                parts.push(part_value);
            }
            AssistantPart::ToolResult(ToolResultPart {
                tool_name,
                output,
                provider_options,
                ..
            }) => {
                let result = match output {
                    ToolResultOutput::Json { value, .. } => value.clone(),
                    _ => json!({}),
                };
                if tool_name == "code_execution" && matches!(output, ToolResultOutput::Json { .. })
                {
                    parts.push(json!({ "codeExecutionResult": result }));
                    continue;
                }
                // Provider-executed tool result in an assistant message —
                // upstream convert-to-google-messages.ts:518-540.
                // If it carries serverToolCallId + serverToolType, emit as
                // a toolResponse; otherwise skip (upstream returns undefined).
                if let Some(opts) = namespace.read_part(provider_options.as_ref()) {
                    let server_id = opts.get("serverToolCallId").and_then(|v| v.as_str());
                    let server_type = opts.get("serverToolType").and_then(|v| v.as_str());
                    if let (Some(sid), Some(st)) = (server_id, server_type) {
                        let mut part_value = if st == "code_execution" {
                            json!({ "codeExecutionResult": result })
                        } else {
                            json!({
                                "toolResponse": {
                                    "toolType": st,
                                    "response": result,
                                    "id": sid,
                                }
                            })
                        };
                        if let Some(signature) = opts
                            .get("thoughtSignature")
                            .and_then(|value| value.as_str())
                        {
                            part_value["thoughtSignature"] = json!(signature);
                        }
                        parts.push(part_value);
                    }
                }
            }
            AssistantPart::ReasoningFile(part) => {
                if let aimux_core::shared::GeneratedFileData::Data { data } = &part.data {
                    let data = match data {
                        FileBytes::Binary(bytes) => {
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        }
                        FileBytes::Base64(data) => data.clone(),
                    };
                    let mut value = json!({ "inlineData": { "mimeType": part.media_type, "data": data }, "thought": true });
                    if let Some(signature) = namespace
                        .read_part(part.provider_options.as_ref())
                        .and_then(|options| options.get("thoughtSignature"))
                    {
                        value["thoughtSignature"] = signature.clone();
                    }
                    parts.push(value);
                }
            }
            AssistantPart::Custom(_) => {}
            AssistantPart::File(file) => {
                let mut value = convert_file_part(file);
                if let Some(opts) = namespace.read_part(file.provider_options.as_ref()) {
                    if opts.get("thought") == Some(&Value::Bool(true)) {
                        value["thought"] = json!(true);
                    }
                    if let Some(signature) = opts.get("thoughtSignature") {
                        value["thoughtSignature"] = signature.clone();
                    }
                }
                parts.push(value);
            }
        }
    }
    parts
}

/// Convert tool-role content parts into Google `functionResponse` parts.
///
/// The TS SDK uses the `functionResponse` shape:
/// `{ functionResponse: { id?, name, response: { name, content } } }`.
/// `content` preserves the tool output unless it contains a JSON Schema `$ref`.
pub(crate) fn tool_file_media_type(file: &FilePart) -> Result<String, aimux_core::AiMuxError> {
    aimux_provider_utils::resolve_full_media_type(file)
}

fn convert_tool_parts(
    content: &[ToolPart],
    namespace: Namespace,
    supports_function_response_parts: bool,
) -> Vec<Value> {
    let mut parts = Vec::new();
    for part in content {
        let ToolPart::ToolResult(ToolResultPart {
            tool_call_id,
            tool_name,
            output,
            ..
        }) = part
        else {
            continue;
        };
        let response = |content: Value| {
            let mut value = json!({ "functionResponse": { "name": tool_name, "response": { "name": tool_name, "content": content } } });
            if namespace != Namespace::Vertex && !tool_call_id.is_empty() {
                value["functionResponse"]["id"] = json!(tool_call_id);
            }
            value
        };
        match output {
            ToolResultOutput::Content { value } => {
                if supports_function_response_parts {
                    let mut texts = Vec::new();
                    let mut files = Vec::new();
                    for part in value {
                        match part {
                            ToolResultContent::Text(part) => texts.push(part.text.clone()),
                            ToolResultContent::File(file) => match &file.data {
                                FileData::Data { data } => {
                                    let data = match data {
                                        FileBytes::Binary(bytes) => {
                                            base64::engine::general_purpose::STANDARD.encode(bytes)
                                        }
                                        FileBytes::Base64(data) => data.clone(),
                                    };
                                    files.push(json!({ "inlineData": { "mimeType": tool_file_media_type(file).unwrap_or_else(|_| file.media_type.clone()), "data": data } }));
                                }
                                FileData::Url { url, .. } if url.starts_with("data:") => {
                                    if let Some((media_type, data)) = url
                                        .strip_prefix("data:")
                                        .and_then(|url| url.split_once(";base64,"))
                                        .filter(|(media_type, data)| {
                                            !media_type.is_empty()
                                                && !media_type.contains([';', ','])
                                                && !data.is_empty()
                                        })
                                    {
                                        files.push(json!({ "inlineData": { "mimeType": media_type, "data": data } }));
                                    } else {
                                        texts.push(
                                            crate::openai::convert::tool_result_content_value(
                                                std::slice::from_ref(part),
                                            )[0]
                                            .to_string(),
                                        );
                                    }
                                }
                                FileData::Url { url, original_url }
                                    if namespace == Namespace::Vertex
                                        && url.starts_with("gs:")
                                        && original_url
                                            .as_deref()
                                            .unwrap_or(url)
                                            .starts_with("gs://")
                                        && matches!(
                                            file.media_type.as_str(),
                                            "image/png"
                                                | "image/jpeg"
                                                | "image/webp"
                                                | "application/pdf"
                                                | "text/plain"
                                        ) =>
                                {
                                    files.push(json!({ "fileData": { "mimeType": file.media_type, "fileUri": original_url.as_deref().unwrap_or(url) } }));
                                }
                                _ => texts.push(
                                    crate::openai::convert::tool_result_content_value(
                                        std::slice::from_ref(part),
                                    )[0]
                                    .to_string(),
                                ),
                            },
                            _ => texts.push(
                                crate::openai::convert::tool_result_content_value(
                                    std::slice::from_ref(part),
                                )[0]
                                .to_string(),
                            ),
                        }
                    }
                    let mut result = response(json!(if texts.is_empty() {
                        "Tool executed successfully.".to_string()
                    } else {
                        texts.join("\n")
                    }));
                    if !files.is_empty() {
                        result["functionResponse"]["parts"] = json!(files);
                    }
                    parts.push(result);
                    continue;
                }
                for part in value {
                    match part {
                        ToolResultContent::Text(part) => parts.push(response(json!(part.text))),
                        ToolResultContent::File(file) => if let FileData::Data { data } = &file.data {
                            let data = match data { FileBytes::Binary(bytes) => base64::engine::general_purpose::STANDARD.encode(bytes), FileBytes::Base64(data) => data.clone() };
                            parts.push(json!({ "inlineData": { "mimeType": tool_file_media_type(file).unwrap_or_else(|_| file.media_type.clone()), "data": data } }));
                            parts.push(json!({ "text": format!("Tool executed successfully and returned this {} as a response", if file.media_type.starts_with("image/") { "image" } else { "file" }) }));
                        } else { parts.push(json!({ "text": crate::openai::convert::tool_result_content_value(std::slice::from_ref(part))[0].to_string() })); },
                        _ => parts.push(json!({ "text": crate::openai::convert::tool_result_content_value(std::slice::from_ref(part))[0].to_string() })),
                    }
                }
            }
            ToolResultOutput::Text { value, .. } | ToolResultOutput::ErrorText { value, .. } => {
                parts.push(response(json!(value)))
            }
            ToolResultOutput::Json { value, .. } | ToolResultOutput::ErrorJson { value, .. } => {
                parts.push(response(if contains_schema_reference(value) {
                    json!(value.to_string())
                } else {
                    value.clone()
                }))
            }
            ToolResultOutput::ExecutionDenied { reason, .. } => parts.push(response(json!(
                reason.as_deref().unwrap_or("Tool call execution denied.")
            ))),
        }
    }
    parts
}

// ── prepareTools ─────────────────────────────────────────────────────────────

/// Result of preparing tools for the Google request body.
#[derive(Debug, Clone, Default)]
pub struct PreparedTools {
    /// `tools` array (e.g. `[{ functionDeclarations: [...] }]`), or `None`
    /// when there are no usable tools.
    pub tools: Option<Vec<Value>>,
    /// `toolConfig` object (e.g. `{ functionCallingConfig: { mode: "AUTO" } }`).
    pub tool_config: Option<Value>,
}

/// Prepare `FunctionTool`s into the Google `tools` / `toolConfig` JSON shape.
///
/// Mirrors the function-tools path of the upstream `prepareTools`.
/// Use [`prepare_all_tools`] for provider-defined tools.
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&ToolChoice>,
) -> PreparedTools {
    // Coerce empty arrays to None (matches TS `tools?.length ? tools : undefined`).
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());

    let Some(tools) = non_empty else {
        return PreparedTools::default();
    };

    let mut has_strict = false;
    let function_declarations: Vec<Value> = tools
        .iter()
        .map(|t| {
            if t.strict == Some(true) {
                has_strict = true;
            }
            build_function_declaration(t)
        })
        .collect();

    let tools_value = Some(vec![
        json!({ "functionDeclarations": function_declarations }),
    ]);

    let tool_config = match tool_choice {
        None => has_strict.then(|| json!({ "functionCallingConfig": { "mode": "VALIDATED" } })),
        Some(ToolChoice::Auto) => {
            if has_strict {
                Some(json!({ "functionCallingConfig": { "mode": "VALIDATED" } }))
            } else {
                Some(json!({ "functionCallingConfig": { "mode": "AUTO" } }))
            }
        }
        Some(ToolChoice::None) => Some(json!({ "functionCallingConfig": { "mode": "NONE" } })),
        Some(ToolChoice::Required) => Some(json!({ "functionCallingConfig": { "mode": "ANY" } })),
        Some(ToolChoice::Tool { tool_name }) => Some(json!({
            "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [tool_name] }
        })),
    };

    PreparedTools {
        tools: tools_value,
        tool_config,
    }
}

// ── Model capabilities ───────────────────────────────────────────────────────

/// Gemini model capabilities, mirroring `getGoogleModelCapabilities` in the TS
/// SDK. Determines which provider-defined tools a model supports and whether the
/// Gemini 3 combined tool shape applies.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoogleModelCapabilities {
    /// `google_search` / `url_context` / `code_execution` / `google_maps` /
    /// `enterprise_web_search` / `vertex_rag_store` require Gemini 2.0+.
    pub supports_gemini_2_tools: bool,
    /// `file_search` requires Gemini 2.5+ or Gemini 3.
    pub supports_file_search: bool,
    /// Gemini 3 keeps function + provider tools together with
    /// `includeServerSideToolInvocations`.
    pub uses_gemini_3_features: bool,
}

/// Classify Gemini capabilities by model id.
///
/// Mirrors `getGoogleModelCapabilities`. Unrecognized Gemini ids inherit the
/// newest supported behaviour (matching the TS intent); only known older
/// generations are downgraded.
#[must_use]
pub fn get_google_model_capabilities(model_id: &str) -> GoogleModelCapabilities {
    let lower = model_id.to_lowercase();

    let is_gemini_model = contains_at_boundary(&lower, "gemini-");
    let is_gemini_2 = matches_prefix_boundary(&lower, "gemini-2");
    let is_gemini_25 = matches_prefix_boundary(&lower, "gemini-2.5");
    let is_gemini_1 = matches_prefix_boundary(&lower, "gemini-1");
    let is_known_pre_gemini_2 = is_gemini_1
        || matches_at_end(&lower, "gemini-pro")
        || matches_at_end(&lower, "gemini-pro-vision")
        || matches_prefix_boundary(&lower, "gemini-robotics-er-1.5");
    let is_known_older_model = is_known_pre_gemini_2 || is_gemini_2;
    let uses_gemini_3_features = is_gemini_model && !is_known_older_model;

    GoogleModelCapabilities {
        supports_gemini_2_tools: (is_gemini_model && !is_known_pre_gemini_2)
            || lower.contains("nano-banana"),
        supports_file_search: is_gemini_25 || uses_gemini_3_features,
        uses_gemini_3_features,
    }
}

/// `/(^|\/)prefix/` — `prefix` appears at the start of `lower` or just after a
/// `/`, with no requirement on what follows.
fn contains_at_boundary(lower: &str, prefix: &str) -> bool {
    lower.starts_with(prefix) || lower.contains(&format!("/{prefix}"))
}

/// `/(^|\/)prefix(?:[.-]|$)/` — `prefix` appears at the start of `lower` or
/// just after a `/`, and is followed by `.`, `-`, or end-of-string.
fn matches_prefix_boundary(lower: &str, prefix: &str) -> bool {
    let bytes = lower.as_bytes();
    let plen = prefix.len();
    if plen > bytes.len() {
        return false;
    }
    let pb = prefix.as_bytes();
    for i in 0..=bytes.len() - plen {
        if i != 0 && bytes[i - 1] != b'/' {
            continue;
        }
        if &bytes[i..i + plen] != pb {
            continue;
        }
        let after = i + plen;
        if after >= bytes.len() {
            return true;
        }
        let next = bytes[after];
        if next == b'.' || next == b'-' {
            return true;
        }
    }
    false
}

/// `/(^|\/)suffix$/` — `lower` ends with `suffix`, and the suffix begins at the
/// start of the string or just after a `/`.
fn matches_at_end(lower: &str, suffix: &str) -> bool {
    let bytes = lower.as_bytes();
    let slen = suffix.len();
    if slen > bytes.len() {
        return false;
    }
    let start = bytes.len() - slen;
    if &bytes[start..] != suffix.as_bytes() {
        return false;
    }
    start == 0 || bytes[start - 1] == b'/'
}

// ── prepareTools (provider-defined tools) ────────────────────────────────────

/// Result of preparing tools for the Google request body, including any
/// warnings about unsupported tools.
#[derive(Debug, Clone, Default)]
pub struct PreparedToolsWithWarnings {
    /// `tools` array, or `None` when there are no usable tools.
    pub tools: Option<Vec<Value>>,
    /// `toolConfig` object.
    pub tool_config: Option<Value>,
    /// Warnings about unsupported tools / combinations.
    pub warnings: Vec<Warning>,
}

/// Prepare function **and** provider-defined tools into the Google `tools` /
/// `toolConfig` JSON shape, mirroring the TS `prepareTools`.
///
/// Unlike [`prepare_tools`] (which only handles `FunctionTool`s), this handles
/// `Tool::Provider` entries (`google.google_search`, `google.code_execution`,
/// `google.url_context`, `google.google_maps`, `google.enterprise_web_search`,
/// `google.file_search`) and the Gemini 3 combined function+provider shape.
#[must_use]
pub fn prepare_all_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
    model_id: &str,
) -> PreparedToolsWithWarnings {
    let caps = get_google_model_capabilities(model_id);
    let mut warnings: Vec<Warning> = Vec::new();

    // Coerce empty arrays to None (matches TS `tools?.length ? tools : undefined`).
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());
    let Some(tools) = non_empty else {
        return PreparedToolsWithWarnings::default();
    };

    let has_function_tools = tools.iter().any(|t| matches!(t, Tool::Function(_)));
    let has_provider_tools = tools.iter().any(|t| matches!(t, Tool::Provider(_)));

    if has_function_tools && has_provider_tools && !caps.uses_gemini_3_features {
        warnings.push(Warning::Unsupported {
            feature: "combination of function and provider-defined tools".to_string(),
            details: None,
        });
    }

    if has_provider_tools {
        let mut google_tools: Vec<Value> = Vec::new();

        for tool in tools.iter().filter_map(|t| match t {
            Tool::Provider(pt) => Some(pt),
            _ => None,
        }) {
            push_provider_tool(tool, &caps, &mut google_tools, &mut warnings);
        }

        // Gemini 3: keep function declarations alongside provider tools.
        if has_function_tools && caps.uses_gemini_3_features && !google_tools.is_empty() {
            let function_declarations: Vec<Value> = tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => Some(build_function_declaration(ft)),
                    _ => None,
                })
                .collect();

            let mut combined_config = Map::new();
            let fc = match tool_choice {
                Some(ToolChoice::None) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("NONE"));
                    m
                }
                Some(ToolChoice::Required) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("ANY"));
                    m
                }
                Some(ToolChoice::Tool { tool_name }) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("ANY"));
                    m.insert("allowedFunctionNames".to_string(), json!([tool_name]));
                    m
                }
                None | Some(ToolChoice::Auto) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("VALIDATED"));
                    m
                }
            };
            combined_config.insert("functionCallingConfig".to_string(), Value::Object(fc));
            combined_config.insert("includeServerSideToolInvocations".to_string(), json!(true));

            let mut tools_value = google_tools;
            tools_value.push(json!({ "functionDeclarations": function_declarations }));

            return PreparedToolsWithWarnings {
                tools: Some(tools_value),
                tool_config: Some(Value::Object(combined_config)),
                warnings,
            };
        }

        let tools_value = if google_tools.is_empty() {
            None
        } else {
            Some(google_tools)
        };
        return PreparedToolsWithWarnings {
            tools: tools_value,
            tool_config: None,
            warnings,
        };
    }

    // Function-only path: delegate to the existing `prepare_tools`.
    let function_tools: Vec<FunctionTool> = tools
        .iter()
        .filter_map(|t| match t {
            Tool::Function(ft) => Some(ft.clone()),
            _ => None,
        })
        .collect();
    let prepared = prepare_tools(&Some(function_tools), tool_choice);
    PreparedToolsWithWarnings {
        tools: prepared.tools,
        tool_config: prepared.tool_config,
        warnings,
    }
}

/// Map a single provider-defined tool to its Google request-body entry, pushing
/// either the tool object or an `Unsupported` warning.
fn push_provider_tool(
    tool: &aimux_core::tool::ProviderTool,
    caps: &GoogleModelCapabilities,
    google_tools: &mut Vec<Value>,
    warnings: &mut Vec<Warning>,
) {
    let unsupported = |details: Option<&str>| Warning::Unsupported {
        feature: format!("provider-defined tool {}", tool.id),
        details: details.map(std::string::ToString::to_string),
    };
    match tool.id.as_str() {
        "google.google_search" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "googleSearch": tool.args }));
            } else {
                warnings.push(unsupported(Some(
                    "Google Search requires Gemini 2.0 or newer.",
                )));
            }
        }
        "google.enterprise_web_search" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "enterpriseWebSearch": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "Enterprise Web Search requires Gemini 2.0 or newer.",
                )));
            }
        }
        "google.url_context" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "urlContext": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The URL context tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.code_execution" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "codeExecution": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The code execution tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.file_search" => {
            if caps.supports_file_search {
                google_tools.push(json!({ "fileSearch": tool.args }));
            } else {
                warnings.push(unsupported(Some(
                    "The file search tool is only supported with Gemini 2.5 models and Gemini 3 models.",
                )));
            }
        }
        "google.vertex_rag_store" => {
            if caps.supports_gemini_2_tools {
                let mut rag =
                    json!({ "rag_resources": { "rag_corpus": tool.args.get("ragCorpus") } });
                if let Some(top_k) = tool.args.get("topK") {
                    rag["similarity_top_k"] = top_k.clone();
                }
                google_tools.push(json!({ "retrieval": { "vertex_rag_store": rag } }));
            } else {
                warnings.push(unsupported(Some(
                    "The RAG store tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.google_maps" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "googleMaps": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The Google Maps grounding tool is not supported with Gemini models other than Gemini 2 or newer.",
                )));
            }
        }
        _ => {
            warnings.push(unsupported(None));
        }
    }
}

/// Build a single `functionDeclarations` entry from a `FunctionTool`.
///
/// The tool's input schema goes out as `parametersJsonSchema`, unchanged, the
/// way `@ai-sdk/google` sends it (Gemini accepts JSON Schema natively). An
/// empty root object schema is preserved.
fn build_function_declaration(ft: &FunctionTool) -> Value {
    let mut decl = Map::new();
    decl.insert("name".to_string(), json!(ft.name));
    decl.insert(
        "description".to_string(),
        json!(ft.description.as_deref().unwrap_or("")),
    );
    decl.insert("parametersJsonSchema".to_string(), ft.input_schema.clone());
    Value::Object(decl)
}

fn contains_schema_reference(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| key == "$ref" || contains_schema_reference(value)),
        Value::Array(array) => array.iter().any(contains_schema_reference),
        _ => false,
    }
}

/// Preserve JSON Schema while replacing unsupported `const` constraints.
#[must_use]
pub fn sanitize_response_json_schema(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return schema.clone();
    };
    let mut result = object.clone();
    if let Some(value) = result.remove("const") {
        result.insert("enum".into(), json!([value]));
    }
    for key in ["properties", "$defs"] {
        if let Some(Value::Object(definitions)) = result.get_mut(key) {
            for value in definitions.values_mut() {
                *value = sanitize_response_json_schema(value);
            }
        }
    }
    for key in ["items", "additionalProperties", "anyOf", "oneOf"] {
        if let Some(value) = result.get_mut(key) {
            if let Value::Array(array) = value {
                for item in array {
                    *item = sanitize_response_json_schema(item);
                }
            } else {
                *value = sanitize_response_json_schema(value);
            }
        }
    }
    Value::Object(result)
}

// ── build_request_body ───────────────────────────────────────────────────────

/// Build the Gemini `generateContent` request body from `CallOptions`.
///
/// Mirrors `getArgs` in `google-language-model.ts`: the sampling settings, the
/// response format, and the provider options the SDK maps (`thinkingConfig`,
/// `responseModalities`, `audioTimestamp`, `mediaResolution`, `imageConfig`
/// into `generationConfig`; `safetySettings`, `cachedContent`, `labels`,
/// `serviceTier` and `retrievalConfig` next to it).
///
/// This is the request-body-only entry point; warnings about unsupported tools
/// are discarded. Use [`build_request_body_with_warnings`] to surface them.
///
/// # Errors
/// Returns an error when `structuredOutputs` is not a boolean.
pub fn build_request_body(model_id: &str, options: &CallOptions) -> Result<Value, AiMuxError> {
    build_request_body_with_warnings(model_id, options).map(|(body, _)| body)
}

/// Build a Vertex Gemini request body using Vertex's provider-metadata
/// namespaces when replaying response parts.
///
/// # Errors
/// Returns an error when `structuredOutputs` is not a boolean.
pub(crate) fn build_vertex_request_body(
    model_id: &str,
    options: &CallOptions,
) -> Result<Value, AiMuxError> {
    build_request_body_with_warnings_for_namespace(model_id, options, Namespace::Vertex)
        .map(|(body, _)| body)
}

/// Build the Gemini `generateContent` request body **and** collect the tool
/// warnings (e.g. unsupported provider-defined tools, mixed function+provider
/// tools on pre-Gemini-3 models).
///
/// # Errors
/// Returns an error when `structuredOutputs` is not a boolean.
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    build_request_body_with_warnings_for_namespace(model_id, options, Namespace::Google)
}

fn build_request_body_with_warnings_for_namespace(
    model_id: &str,
    options: &CallOptions,
    namespace: Namespace,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    let provider_options = options.provider_options.as_ref().and_then(|options| {
        namespace
            .read_keys()
            .iter()
            .find_map(|name| options.get(*name).map(|value| (*name, value)))
    });
    let structured_outputs =
        match provider_options.and_then(|(_, options)| options.get("structuredOutputs")) {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(AiMuxError::InvalidArgument(format!(
                    "invalid {} provider options",
                    provider_options.unwrap().0
                )));
            }
        };
    let GooglePrompt {
        mut system_instruction,
        mut contents,
    } = convert_to_google_messages_for_namespace(
        &options.prompt,
        namespace,
        get_google_model_capabilities(model_id).uses_gemini_3_features,
    );
    let mut warnings = Vec::new();
    let provider_options = namespace.read_in(options.provider_options.as_ref());
    let option = |name: &str| {
        provider_options
            .and_then(|o| o.get(name))
            .filter(|v| !v.is_null())
    };
    let is_vertex = namespace == Namespace::Vertex;
    if !is_vertex
        && options.tools.as_ref().is_some_and(|tools| {
            tools.iter().any(
                |tool| matches!(tool, Tool::Provider(tool) if tool.id == "google.vertex_rag_store"),
            )
        })
    {
        warnings.push(Warning::Other { message: "The 'vertex_rag_store' tool is only supported with the Google Vertex provider and might not be supported or could behave unexpectedly with the current Google provider (google.generative-ai).".into() });
    }

    if !is_vertex && option("streamFunctionCallArguments") == Some(&Value::Bool(true)) {
        warnings.push(Warning::Other { message: "'streamFunctionCallArguments' is only supported on the Vertex AI API and will be ignored with the current Google provider (google.generative-ai). See https://docs.cloud.google.com/vertex-ai/generative-ai/docs/multimodal/function-calling#streaming-fc".into() });
    }
    if is_vertex && option("serviceTier").is_some() {
        warnings.push(Warning::Other { message: "'serviceTier' is a Gemini API option and is not supported on Vertex AI. Use 'sharedRequestType' (and optionally 'requestType') instead. See https://docs.cloud.google.com/vertex-ai/generative-ai/docs/priority-paygo".into() });
    }
    if !is_vertex && (option("sharedRequestType").is_some() || option("requestType").is_some()) {
        warnings.push(Warning::Other { message: "'sharedRequestType' and 'requestType' are Vertex AI options and are ignored with the current Google provider (google.generative-ai).".into() });
    }
    let option_warning_count = warnings.len();
    let mut prompt_warnings = Vec::new();
    let mut thinking_warnings = Vec::new();
    let omit_penalties =
        !is_vertex && matches_prefix_boundary(&model_id.to_lowercase(), "gemini-2.5");
    if model_id.to_lowercase().starts_with("gemma-")
        && let Some(system) = system_instruction.take()
        && let Some(first) = contents.first_mut()
        && first["role"] == "user"
        && let Some(parts) = first["parts"].as_array_mut()
    {
        let texts = system["parts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        parts.insert(0, json!({ "text": format!("{texts}\n\n") }));
    }
    if get_google_model_capabilities(model_id).uses_gemini_3_features {
        let mut missing = Vec::new();
        for content in &mut contents {
            if content["role"] != "model" {
                continue;
            }
            let mut has_signed_call = false;
            for part in content["parts"].as_array_mut().unwrap() {
                let standard = part.get("functionCall").is_some();
                let server = part.get("toolCall").is_some();
                if !standard && !server {
                    continue;
                }
                if part.get("thoughtSignature").is_some() {
                    if standard {
                        has_signed_call = true;
                    }
                } else if server || !has_signed_call {
                    part["thoughtSignature"] = json!("skip_thought_signature_validator");
                    missing.push(
                        part.get("functionCall")
                            .or_else(|| part.get("toolCall"))
                            .unwrap()
                            .get("name")
                            .or_else(|| part["toolCall"].get("toolType"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    );
                }
            }
        }
        if !missing.is_empty() {
            let mut names = Vec::new();
            for name in &missing {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            prompt_warnings.push(Warning::Other { message: format!(
                "Replayed {} `functionCall` part(s) for a Gemini 3 model without a `thoughtSignature` (tools: {}). Injected the documented `skip_thought_signature_validator` sentinel to keep the request from failing with HTTP 400. The likely cause is application code that drops `providerOptions.google.thoughtSignature` when persisting or serializing assistant tool-call messages. See https://ai.google.dev/gemini-api/docs/thought-signatures.",
                missing.len(), names.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ")) });
        }
    }

    let mut generation_config = Map::new();

    if let Some(max_tokens) = options.max_output_tokens {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = options.temperature {
        generation_config.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        generation_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(top_k) = options.top_k {
        generation_config.insert("topK".to_string(), json!(top_k));
    }
    for (name, value) in [
        ("frequencyPenalty", options.frequency_penalty),
        ("presencePenalty", options.presence_penalty),
    ] {
        if let Some(value) = value {
            if omit_penalties {
                warnings.push(Warning::Unsupported {
                    feature: name.into(),
                    details: None,
                });
            } else {
                generation_config.insert(name.into(), json!(value));
            }
        }
    }
    if let Some(stop) = &options.stop_sequences {
        generation_config.insert("stopSequences".to_string(), json!(stop));
    }
    if let Some(seed) = options.seed {
        generation_config.insert("seed".to_string(), json!(seed));
    }

    // JSON output uses responseMimeType and the sanitized responseJsonSchema.
    if let Some(rf) = &options.response_format
        && let ResponseFormat::Json { schema, .. } = rf
    {
        generation_config.insert("responseMimeType".to_string(), json!("application/json"));
        if let Some(schema) = schema
            && structured_outputs
        {
            generation_config.insert(
                "responseJsonSchema".into(),
                sanitize_response_json_schema(schema),
            );
        }
    }

    // Provider options (`providerOptions.google`, or `googleVertex` for
    // Vertex): the generation-config members, then the top-level ones.
    for name in ["responseModalities", "audioTimestamp", "mediaResolution"] {
        if let Some(value) = option(name)
            && (name != "audioTimestamp" || value == &Value::Bool(true))
        {
            generation_config.insert(name.into(), value.clone());
        }
    }
    let mut thinking = resolve_thinking(options.reasoning, model_id, &mut thinking_warnings);
    if let Some(explicit) = option("thinkingConfig").and_then(Value::as_object) {
        thinking
            .get_or_insert_with(Map::new)
            .extend(explicit.clone());
    }
    if let Some(thinking) = thinking {
        generation_config.insert("thinkingConfig".into(), Value::Object(thinking));
    }
    if let Some(image) = option("imageConfig") {
        let mut image = image.clone();
        if !is_vertex && let Some(config) = image.as_object_mut() {
            let mut dropped = Vec::new();
            for name in ["personGeneration", "prominentPeople", "imageOutputOptions"] {
                if config.remove(name).is_some() {
                    dropped.push(format!("'imageConfig.{name}'"));
                }
            }
            if !dropped.is_empty() {
                warnings.insert(option_warning_count, Warning::Other {
                    message: format!(
                        "{} {} ignored with the current Google provider (google.generative-ai).",
                        dropped.join(", "),
                        if dropped.len() == 1 {
                            "is a Vertex AI option and is"
                        } else {
                            "are Vertex AI options and are"
                        }
                    ),
                });
            }
        }
        generation_config.insert("imageConfig".into(), image);
    }
    for (name, keys) in [
        (
            "thinkingConfig",
            &["thinkingBudget", "includeThoughts", "thinkingLevel"][..],
        ),
        (
            "imageConfig",
            &[
                "aspectRatio",
                "imageSize",
                "personGeneration",
                "prominentPeople",
                "imageOutputOptions",
            ][..],
        ),
    ] {
        if let Some(Value::Object(config)) = generation_config.get_mut(name) {
            config.retain(|key, _| keys.contains(&key.as_str()));
            if let Some(Value::Object(output)) = config.get_mut("imageOutputOptions") {
                output.retain(|key, _| ["mimeType", "compressionQuality"].contains(&key.as_str()));
            }
        }
    }
    let mut body = Map::new();
    body.insert("contents".to_string(), Value::Array(contents));
    if let Some(sys) = system_instruction {
        body.insert("systemInstruction".to_string(), sys);
    }
    // Always present, even when empty (`generationConfig: {}`), like the SDK.
    body.insert(
        "generationConfig".to_string(),
        Value::Object(generation_config),
    );
    for name in ["safetySettings", "cachedContent", "labels", "serviceTier"] {
        if let Some(value) = option(name)
            && !(is_vertex && name == "serviceTier")
        {
            body.insert(name.to_string(), value.clone());
        }
    }
    if option("safetySettings").is_none()
        && let Some(threshold) = option("threshold")
    {
        body.insert(
            "safetySettings".into(),
            json!(
                [
                    "HARM_CATEGORY_HATE_SPEECH",
                    "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "HARM_CATEGORY_HARASSMENT",
                    "HARM_CATEGORY_SEXUALLY_EXPLICIT"
                ]
                .map(|category| json!({ "category": category, "threshold": threshold }))
            ),
        );
    }
    if let Some(Value::Array(settings)) = body.get_mut("safetySettings") {
        for setting in settings {
            if let Some(setting) = setting.as_object_mut() {
                setting.retain(|key, _| ["category", "threshold"].contains(&key.as_str()));
            }
        }
    }

    let prepared = prepare_all_tools(&options.tools, options.tool_choice.as_ref(), model_id);
    if let Some(tools) = prepared.tools {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    let mut tool_config = prepared.tool_config;
    if is_vertex && let Some(Value::Object(config)) = tool_config.as_mut() {
        config.remove("includeServerSideToolInvocations");
    }
    if let Some(retrieval) = option("retrievalConfig") {
        let mut config = match tool_config.take() {
            Some(Value::Object(config)) => config,
            _ => Map::new(),
        };
        let mut retrieval = retrieval.clone();
        if let Some(retrieval) = retrieval.as_object_mut() {
            retrieval.retain(|key, _| key == "latLng");
            if let Some(Value::Object(coordinates)) = retrieval.get_mut("latLng") {
                coordinates.retain(|key, _| ["latitude", "longitude"].contains(&key.as_str()));
            }
        }
        config.insert("retrievalConfig".to_string(), retrieval);
        tool_config = Some(Value::Object(config));
    }
    if let Some(tc) = tool_config {
        body.insert("toolConfig".to_string(), tc);
    }

    // Model id is *not* part of the body for Gemini — it's in the URL path
    // (`models/{model}:generateContent`). We don't emit it here, matching
    // the TS SDK. (Some callers include it; the API ignores extra fields.)
    let _ = model_id;

    warnings.extend(prompt_warnings);
    warnings.extend(thinking_warnings);
    warnings.extend(prepared.warnings);
    Ok((Value::Object(body), warnings))
}

/// Validate prompt constraints and provider options before sending a request.
pub(crate) fn validate_call_options(
    options: &CallOptions,
) -> Result<(), aimux_core::error::AiMuxError> {
    validate_call_options_for_namespace(options, Namespace::Google)
}

pub(crate) fn validate_call_options_for_namespace(
    options: &CallOptions,
    namespace: Namespace,
) -> Result<(), aimux_core::error::AiMuxError> {
    use aimux_core::error::AiMuxError;
    let mut initial = true;
    for message in &options.prompt {
        if matches!(message, LanguageModelMessage::System { .. }) {
            if !initial {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "system messages are only supported at the beginning of the conversation"
                        .into(),
                ));
            }
        } else {
            initial = false;
        }
        match message {
            LanguageModelMessage::User { content, .. } => {
                for part in content {
                    if let UserPart::File(file) = part {
                        if namespace == Namespace::Vertex
                            && matches!(file.data, FileData::Reference { .. })
                        {
                            return Err(AiMuxError::UnsupportedFunctionality(
                                "file parts with provider references".into(),
                            ));
                        }
                        validate_file_part(file, false)?;
                    }
                }
            }
            LanguageModelMessage::Assistant { content, .. } => {
                for part in content {
                    match part {
                        AssistantPart::File(file) => {
                            if namespace == Namespace::Vertex
                                && matches!(file.data, FileData::Reference { .. })
                            {
                                return Err(AiMuxError::UnsupportedFunctionality(
                                    "file parts with provider references".into(),
                                ));
                            }
                            validate_file_part(file, true)?;
                        }
                        AssistantPart::ReasoningFile(file)
                            if matches!(
                                file.data,
                                aimux_core::shared::GeneratedFileData::Url { .. }
                            ) =>
                        {
                            return Err(AiMuxError::UnsupportedFunctionality(
                                "File data URLs in assistant messages are not supported".into(),
                            ));
                        }
                        _ => {}
                    }
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    if let ToolPart::ToolResult(part) = part
                        && let ToolResultOutput::Content { value } = &part.output
                    {
                        for part in value {
                            if let ToolResultContent::File(file) = part
                                && matches!(file.data, FileData::Data { .. })
                            {
                                aimux_provider_utils::resolve_full_media_type(file)?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let Some(values) = Namespace::Google.read_in(options.provider_options.as_ref()) else {
        return Ok(());
    };
    let invalid =
        |name: &str| AiMuxError::InvalidArgument(format!("Invalid Google provider option: {name}"));
    for name in [
        "structuredOutputs",
        "audioTimestamp",
        "streamFunctionCallArguments",
    ] {
        if values.get(name).is_some_and(|value| !value.is_boolean()) {
            return Err(invalid(name));
        }
    }
    if values
        .get("cachedContent")
        .is_some_and(|value| !value.is_string())
    {
        return Err(invalid("cachedContent"));
    }
    for (name, allowed) in [
        (
            "threshold",
            &[
                "HARM_BLOCK_THRESHOLD_UNSPECIFIED",
                "BLOCK_LOW_AND_ABOVE",
                "BLOCK_MEDIUM_AND_ABOVE",
                "BLOCK_ONLY_HIGH",
                "BLOCK_NONE",
                "OFF",
            ][..],
        ),
        (
            "mediaResolution",
            &[
                "MEDIA_RESOLUTION_UNSPECIFIED",
                "MEDIA_RESOLUTION_LOW",
                "MEDIA_RESOLUTION_MEDIUM",
                "MEDIA_RESOLUTION_HIGH",
            ][..],
        ),
        ("serviceTier", &["standard", "flex", "priority"][..]),
        ("sharedRequestType", &["priority", "flex", "standard"][..]),
        ("requestType", &["shared"][..]),
    ] {
        if values
            .get(name)
            .is_some_and(|value| !value.as_str().is_some_and(|value| allowed.contains(&value)))
        {
            return Err(invalid(name));
        }
    }
    if values.get("responseModalities").is_some_and(|value| {
        !value.as_array().is_some_and(|items| {
            items
                .iter()
                .all(|item| matches!(item.as_str(), Some("TEXT" | "IMAGE")))
        })
    }) {
        return Err(invalid("responseModalities"));
    }
    if let Some(value) = values.get("thinkingConfig") {
        let config = value.as_object().ok_or_else(|| invalid("thinkingConfig"))?;
        if config
            .get("thinkingBudget")
            .is_some_and(|value| !value.is_number())
            || config
                .get("includeThoughts")
                .is_some_and(|value| !value.is_boolean())
            || config.get("thinkingLevel").is_some_and(|value| {
                !matches!(value.as_str(), Some("minimal" | "low" | "medium" | "high"))
            })
        {
            return Err(invalid("thinkingConfig"));
        }
    }
    if let Some(value) = values.get("labels")
        && !value
            .as_object()
            .is_some_and(|items| items.values().all(Value::is_string))
    {
        return Err(invalid("labels"));
    }
    if let Some(value) = values.get("safetySettings")
        && !value.as_array().is_some_and(|items| {
            items.iter().all(|item| {
                matches!(
                    item["category"].as_str(),
                    Some(
                        "HARM_CATEGORY_UNSPECIFIED"
                            | "HARM_CATEGORY_HATE_SPEECH"
                            | "HARM_CATEGORY_DANGEROUS_CONTENT"
                            | "HARM_CATEGORY_HARASSMENT"
                            | "HARM_CATEGORY_SEXUALLY_EXPLICIT"
                            | "HARM_CATEGORY_CIVIC_INTEGRITY"
                    )
                ) && matches!(
                    item["threshold"].as_str(),
                    Some(
                        "HARM_BLOCK_THRESHOLD_UNSPECIFIED"
                            | "BLOCK_LOW_AND_ABOVE"
                            | "BLOCK_MEDIUM_AND_ABOVE"
                            | "BLOCK_ONLY_HIGH"
                            | "BLOCK_NONE"
                            | "OFF"
                    )
                )
            })
        })
    {
        return Err(invalid("safetySettings"));
    }
    if let Some(value) = values.get("imageConfig") {
        let config = value.as_object().ok_or_else(|| invalid("imageConfig"))?;
        for (name, allowed) in [
            (
                "aspectRatio",
                &[
                    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9", "1:8",
                    "8:1", "1:4", "4:1",
                ][..],
            ),
            ("imageSize", &["1K", "2K", "4K", "512"][..]),
            (
                "personGeneration",
                &[
                    "PERSON_GENERATION_UNSPECIFIED",
                    "ALLOW_ALL",
                    "ALLOW_ADULT",
                    "ALLOW_NONE",
                ][..],
            ),
            (
                "prominentPeople",
                &[
                    "PROMINENT_PEOPLE_UNSPECIFIED",
                    "ALLOW_PROMINENT_PEOPLE",
                    "BLOCK_PROMINENT_PEOPLE",
                ][..],
            ),
        ] {
            if config
                .get(name)
                .is_some_and(|value| !value.as_str().is_some_and(|value| allowed.contains(&value)))
            {
                return Err(invalid(name));
            }
        }
        if let Some(value) = config.get("imageOutputOptions") {
            let output = value
                .as_object()
                .ok_or_else(|| invalid("imageOutputOptions"))?;
            if output
                .get("mimeType")
                .is_some_and(|value| !matches!(value.as_str(), Some("image/jpeg" | "image/png")))
                || output
                    .get("compressionQuality")
                    .is_some_and(|value| !value.is_number())
            {
                return Err(invalid("imageOutputOptions"));
            }
        }
    }
    if let Some(value) = values.get("retrievalConfig") {
        let config = value
            .as_object()
            .ok_or_else(|| invalid("retrievalConfig"))?;
        if let Some(value) = config.get("latLng")
            && !value.as_object().is_some_and(|coordinates| {
                coordinates.get("latitude").is_some_and(Value::is_number)
                    && coordinates.get("longitude").is_some_and(Value::is_number)
            })
        {
            return Err(invalid("retrievalConfig.latLng"));
        }
    }
    Ok(())
}

fn validate_file_part(
    file: &FilePart,
    assistant: bool,
) -> Result<(), aimux_core::error::AiMuxError> {
    use aimux_core::error::AiMuxError;
    if !matches!(file.data, FileData::Text { .. }) {
        aimux_provider_utils::resolve_full_media_type(file)?;
    }
    if assistant && matches!(file.data, FileData::Url { .. }) {
        return Err(AiMuxError::UnsupportedFunctionality(
            "File data URLs in assistant messages are not supported".into(),
        ));
    }
    if let FileData::Reference { reference } = &file.data
        && !reference.contains_key(GOOGLE)
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "No provider reference found for provider 'google'. Available providers: {}",
            reference.keys().cloned().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(())
}

fn resolve_thinking(
    reasoning: Option<aimux_core::types::ReasoningEffort>,
    model_id: &str,
    warnings: &mut Vec<Warning>,
) -> Option<Map<String, Value>> {
    use aimux_core::types::ReasoningEffort as R;
    let reasoning = reasoning.filter(|effort| *effort != R::ProviderDefault)?;
    let lower = model_id.to_lowercase();
    if get_google_model_capabilities(model_id).uses_gemini_3_features
        && !model_id.contains("gemini-3-pro-image")
    {
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        let minimum = if name == "gemini-flash-latest"
            || name
                .strip_prefix("gemini-")
                .and_then(|s| s.split_once("-flash"))
                .is_some_and(|(version, tail)| {
                    let Some((major, minor)) = version.split_once('.') else {
                        return false;
                    };
                    let major = major.parse::<u32>().unwrap_or(0);
                    let minor = minor.parse::<u32>().unwrap_or(0);
                    (tail.is_empty()
                        || (tail.starts_with('-')
                            && tail != "-lite"
                            && !tail.starts_with("-lite-")))
                        && (major > 3 || major == 3 && minor >= 7)
                }) {
            "low"
        } else {
            "minimal"
        };
        let level = match reasoning {
            R::None | R::Minimal => minimum,
            R::Low => "low",
            R::Medium => "medium",
            R::High | R::Xhigh => "high",
            R::ProviderDefault => unreachable!(),
        };
        if reasoning != R::None && level != reasoning.to_string() {
            warnings.push(Warning::Compatibility { feature: "reasoning".into(), details: Some(format!("reasoning \"{reasoning}\" is not directly supported by this model. mapped to effort \"{level}\".")) });
        }
        return Some(Map::from_iter([("thinkingLevel".into(), json!(level))]));
    }
    let percentage = match reasoning {
        R::None => 0.0,
        R::Minimal => 0.02,
        R::Low => 0.1,
        R::Medium => 0.3,
        R::High => 0.6,
        R::Xhigh => 0.9,
        R::ProviderDefault => unreachable!(),
    };
    let max = if lower.contains("2.5-pro") || lower.contains("gemini-3-pro-image") {
        32768
    } else {
        24576
    };
    Some(Map::from_iter([(
        "thinkingBudget".into(),
        json!(((65536.0_f64 * percentage).round() as u32).min(max)),
    )]))
}

// ── finish reason ────────────────────────────────────────────────────────────

/// Map a Gemini `finishReason` string to the unified `FinishReason`.
///
/// `STOP` maps to `ToolCalls` when `has_tool_calls` is true (mirroring the
/// TS `mapGoogleFinishReason`).
#[must_use]
pub fn parse_finish_reason(reason: &str, has_tool_calls: bool) -> FinishReason {
    let unified = match reason {
        "STOP" => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        "MAX_TOKENS" => FinishReasonUnified::Length,
        "IMAGE_SAFETY" | "RECITATION" | "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            FinishReasonUnified::ContentFilter
        }
        "MALFORMED_FUNCTION_CALL" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(reason.to_string()),
    }
}

// ── usage conversion ────────────────────────────────────────────────────────

/// Convert a `GoogleUsageMetadata` into the core `Usage` type.
///
/// Mirrors `convertGoogleUsage`:
/// - `input.total = promptTokenCount + toolUsePromptTokenCount`
/// - `input.noCache = input.total - cachedContentTokenCount`
/// - `input.cacheRead = cachedContentTokenCount`
/// - `output.total = candidatesTokenCount + thoughtsTokenCount`
#[must_use]
pub fn convert_usage(usage: &super::types::GoogleUsageMetadata) -> aimux_core::types::Usage {
    use aimux_core::types::Usage;

    let prompt =
        usage.prompt_token_count.unwrap_or(0) + usage.tool_use_prompt_token_count.unwrap_or(0);
    let candidates = usage.candidates_token_count.unwrap_or(0);
    let cached = usage.cached_content_token_count.unwrap_or(0);
    let thoughts = usage.thoughts_token_count.unwrap_or(0);

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(prompt),
            no_cache: Some(prompt - cached),
            cache_read: Some(cached),
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(candidates + thoughts),
            text: Some(candidates),
            reasoning: Some(thoughts),
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: serde_json::to_value(usage)
            .ok()
            .and_then(|value| value.as_object().cloned()),
    }
}

// ── source extraction ────────────────────────────────────────────────────────

/// Extract `GenerateContent::Source` items from `groundingMetadata.groundingChunks`,
/// mirroring the TS `extractSources`.
///
/// - `web` → url source (`uri`, `title`)
/// - `image` → url source (`sourceUri`, `title`)
/// - `retrievedContext` with http(s) `uri` → url source
/// - `retrievedContext` with non-http `uri` (e.g. `gs://`) → document source
///   (title defaults to "Unknown Document"; `url` is `None`)
/// - `retrievedContext` with `fileSearchStore` (no `uri`) → document source
/// - `maps` → url source (`uri`, `title`)
pub fn extract_sources(
    grounding_metadata: Option<&Value>,
    id_counter: &mut usize,
) -> Vec<GenerateContent> {
    let mut sources = Vec::new();
    let Some(gm) = grounding_metadata else {
        return sources;
    };
    let Some(chunks) = gm.get("groundingChunks").and_then(|c| c.as_array()) else {
        return sources;
    };

    let next_id = |counter: &mut usize| -> String {
        let id = format!("{counter}");
        *counter += 1;
        id
    };

    for chunk in chunks {
        if let Some(web) = chunk.get("web").filter(|value| !value.is_null()) {
            sources.push(GenerateContent::Source(Source::Url {
                id: next_id(id_counter),
                url: web
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: web
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                provider_metadata: None,
            }));
        } else if let Some(image) = chunk.get("image").filter(|value| !value.is_null()) {
            sources.push(GenerateContent::Source(Source::Url {
                id: next_id(id_counter),
                url: image
                    .get("sourceUri")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: image
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                provider_metadata: None,
            }));
        } else if let Some(rc) = chunk
            .get("retrievedContext")
            .filter(|value| !value.is_null())
        {
            let uri = rc
                .get("uri")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let file_search_store = rc
                .get("fileSearchStore")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let title = rc.get("title").and_then(|v| v.as_str());
            if let Some(uri) = uri {
                if uri.starts_with("http://") || uri.starts_with("https://") {
                    sources.push(GenerateContent::Source(Source::Url {
                        id: next_id(id_counter),
                        url: uri.to_string(),
                        title: title.map(std::string::ToString::to_string),
                        provider_metadata: None,
                    }));
                } else {
                    // Document with a file path (gs://, etc.).
                    let media_type = if uri.ends_with(".pdf") {
                        "application/pdf"
                    } else if uri.ends_with(".txt") {
                        "text/plain"
                    } else if uri.ends_with(".docx") {
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    } else if uri.ends_with(".doc") {
                        "application/msword"
                    } else if uri.ends_with(".md") || uri.ends_with(".markdown") {
                        "text/markdown"
                    } else {
                        "application/octet-stream"
                    };
                    sources.push(GenerateContent::Source(Source::Document {
                        id: next_id(id_counter),
                        media_type: media_type.to_string(),
                        title: title.unwrap_or("Unknown Document").to_string(),
                        filename: uri.rsplit('/').next().map(str::to_string),
                        provider_metadata: None,
                    }));
                }
            } else if let Some(file_search_store) = file_search_store {
                // New File Search format (no uri, has fileSearchStore).
                sources.push(GenerateContent::Source(Source::Document {
                    id: next_id(id_counter),
                    media_type: "application/octet-stream".to_string(),
                    title: title.unwrap_or("Unknown Document").to_string(),
                    filename: file_search_store.rsplit('/').next().map(str::to_string),
                    provider_metadata: None,
                }));
            }
            // else: no uri and no fileSearchStore → no source.
        } else if let Some(maps) = chunk.get("maps").filter(|value| !value.is_null())
            && let Some(uri) = maps
                .get("uri")
                .and_then(|v| v.as_str())
                .filter(|uri| !uri.is_empty())
        {
            sources.push(GenerateContent::Source(Source::Url {
                id: next_id(id_counter),
                url: uri.to_string(),
                title: maps
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                provider_metadata: None,
            }));
        }
    }

    sources
}
