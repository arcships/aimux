//! `convert-to-google-messages.ts`: a `LanguageModelPrompt` in Gemini's
//! `contents` + `systemInstruction` shape.
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
use super::prompt::GooglePrompt;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileBytes, FileData};
use base64::Engine;
use serde_json::{Map, Value, json};

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
pub(super) fn convert_to_google_messages_for_namespace(
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

fn contains_schema_reference(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| key == "$ref" || contains_schema_reference(value)),
        Value::Array(array) => array.iter().any(contains_schema_reference),
        _ => false,
    }
}
