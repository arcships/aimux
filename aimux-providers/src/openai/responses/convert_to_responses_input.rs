//! Conversion of a `LanguageModelPrompt` to the Responses `input` items
//! (convert-to-openai-responses-input.ts), with the file id prefixes.

use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::shared::{FileData, SharedProviderOptions};
use aimux_core::tool::Tool;
use aimux_core::types::Warning;
use aimux_core::{AiMuxError, ErrorCause};
use serde_json::{Value, json};

use crate::openai::language_model_capabilities::SystemMessageMode;
use crate::openai::responses::responses_language_model_options::ResponsesNamespace;
use crate::openai::responses::responses_prepare_tools::map_tool_fields;
use crate::openai::responses::tool_args::{validate_tool_call, validate_tool_result};

/// Result of converting a prompt into the Responses API `input` array.
pub struct ResponsesInputResult {
    pub input: Vec<Value>,
    pub warnings: Vec<Warning>,
}

/// Convert a `LanguageModelPrompt` into the OpenAI Responses API `input` array.
///
/// Mirrors the core paths of TS `convertToOpenAIResponsesInput`:
/// - system -> `{ role: "system"|"developer", content: <text> }`
/// - user -> `{ role: "user", content: [{ type: "input_text", text }] }`
/// - assistant text -> `{ role: "assistant", content: [{ type: "output_text", text }] }`
///   (or `{ type: "item_reference", id }` when `store` and an itemId are present)
/// - assistant tool-call -> `{ type: "function_call", call_id, name, arguments }`
/// - tool result -> `{ type: "function_call_output", call_id, output }`
///
/// `store` defaults to `true` (matching the API default). When
/// `has_previous_response_id` is true, assistant reasoning/function-call items
/// that already carry an `itemId` are skipped (they live in the previous
/// response chain).
/// # Errors
/// Rejects unsupported file data and unresolved file references.
pub fn convert_to_responses_input(
    ns: ResponsesNamespace,
    prompt: &LanguageModelPrompt,
    system_message_mode: SystemMessageMode,
    store: bool,
    has_previous_response_id: bool,
) -> Result<ResponsesInputResult, AiMuxError> {
    convert_responses_input_with_conversation(
        ns,
        prompt,
        system_message_mode,
        store,
        has_previous_response_id,
        false,
        None,
    )
}

pub(crate) fn convert_responses_input_with_conversation(
    ns: ResponsesNamespace,
    prompt: &LanguageModelPrompt,
    system_message_mode: SystemMessageMode,
    store: bool,
    has_previous_response_id: bool,
    has_conversation: bool,
    tools: Option<&[Tool]>,
) -> Result<ResponsesInputResult, AiMuxError> {
    let mut input: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    let mut processed_approval_ids = std::collections::HashSet::new();
    let mut programmatic_tool_call_ids = std::collections::HashSet::new();
    for msg in prompt {
        match msg {
            LanguageModelMessage::System {
                content,
                provider_options,
            } => {
                if let Some(effort) =
                    openai_sub_option(ns, provider_options, "reasoningEffortUpdate")
                {
                    input.push(json!({ "type": "configuration_update", "reasoning": { "effort": effort } }));
                    continue;
                }
                match system_message_mode {
                    SystemMessageMode::System => {
                        input.push(json!({
                            "role": "system",
                            "content": system_content(ns, content, provider_options),
                        }));
                    }
                    SystemMessageMode::Developer => {
                        input.push(json!({
                            "role": "developer",
                            "content": system_content(ns, content, provider_options),
                        }));
                    }
                    SystemMessageMode::Remove => {
                        warnings.push(Warning::Other {
                            message: "system messages are removed for this model".to_string(),
                        });
                    }
                }
            }
            LanguageModelMessage::User { content, .. } => {
                let content: Vec<Value> = content
                    .iter()
                    .enumerate()
                    .map(|(index, part)| {
                        let mut value = convert_user_part(ns, part, index)?;
                        if let Some(breakpoint) =
                            openai_sub_option(ns, user_part_options(part), "promptCacheBreakpoint")
                        {
                            value["prompt_cache_breakpoint"] = breakpoint;
                        }
                        Ok(value)
                    })
                    .collect::<Result<_, AiMuxError>>()?;
                input.push(json!({ "role": "user", "content": content }));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                // Reasoning item id → its index in `input`: the parts of one
                // reasoning item become one input item (per assistant
                // message, as `reasoningMessages` upstream).
                let mut reasoning_items: std::collections::HashMap<String, Option<usize>> =
                    std::collections::HashMap::new();
                for part in content {
                    match part {
                        AssistantPart::Text(TextPart {
                            text,
                            provider_options,
                        }) => {
                            let id = item_id(ns, provider_options);
                            if has_conversation && id.is_some() {
                                continue;
                            }
                            if store && let Some(ref id) = id {
                                input.push(json!({ "type": "item_reference", "id": id }));
                                continue;
                            }
                            let phase = phase_from_provider_options(ns, provider_options);
                            let mut item = json!({
                                "role": "assistant",
                                "content": text,
                            });
                            if let Some(phase) = phase {
                                item["phase"] = json!(phase);
                            }
                            input.push(item);
                        }
                        AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            provider_options,
                            provider_executed,
                            ..
                        }) => {
                            let id = item_id(ns, provider_options);
                            if openai_sub_option(ns, provider_options, "caller")
                                .is_some_and(|caller| caller["type"] == "program")
                            {
                                programmatic_tool_call_ids.insert(tool_call_id.as_str());
                            }
                            if has_conversation && id.is_some() {
                                continue;
                            }
                            let kind = provider_tool_kind(tools, tool_name);
                            // Hosted tool search and program calls are replayed
                            // in full without response storage.
                            if provider_executed == &Some(true)
                                && !matches!(
                                    kind,
                                    Some("tool_search" | "programmatic_tool_calling")
                                )
                            {
                                if store && let Some(id) = &id {
                                    input.push(json!({"type":"item_reference", "id":id}));
                                }
                                if store || kind != Some("shell") {
                                    continue;
                                }
                            }
                            if let Some(kind) = kind {
                                if store
                                    && id.is_some()
                                    && kind != "tool_search"
                                    && kind != "programmatic_tool_calling"
                                    && has_previous_response_id
                                {
                                    continue;
                                }
                                if store && let Some(id) = &id {
                                    input.push(json!({"type":"item_reference", "id":id}));
                                    continue;
                                }
                                if let Some(mut item) = provider_tool_call_input(
                                    kind,
                                    tool_name,
                                    tool_call_id,
                                    id.as_deref(),
                                    tool_input,
                                )? {
                                    if kind == "custom"
                                        && let Some(value) =
                                            openai_sub_option(ns, provider_options, "async")
                                                .filter(|value| !value.is_null())
                                    {
                                        item["async"] = value;
                                    }
                                    input.push(item);
                                    continue;
                                }
                            }
                            let namespace = namespace_from_provider_options(ns, provider_options);
                            let mut item = json!({
                                "type": "function_call",
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": serialize_arguments(tool_input),
                            });
                            if let Some(ref namespace) = namespace {
                                item["namespace"] = json!(namespace);
                            }
                            if let Some(value) = openai_sub_option(ns, provider_options, "async") {
                                item["async"] = value;
                            }
                            if let Some(caller) = tool_caller(ns, provider_options) {
                                item["caller"] = caller;
                            }
                            input.push(item);
                        }
                        AssistantPart::Reasoning(ReasoningPart {
                            text,
                            provider_options,
                            ..
                        }) => {
                            // convert-to-openai-responses-input.ts:1151-1250 (openai@4.0.83).
                            let reasoning_id = openai_sub_option(ns, provider_options, "itemId")
                                .and_then(|id| id.as_str().map(str::to_owned));
                            let encrypted = openai_sub_option(
                                ns,
                                provider_options,
                                "reasoningEncryptedContent",
                            )
                            .filter(|value| !value.is_null());
                            if (has_previous_response_id || has_conversation)
                                && reasoning_id.is_some()
                            {
                                continue;
                            }
                            let summary_part = (!text.is_empty())
                                .then(|| json!({ "type": "summary_text", "text": text }));
                            if let Some(rid) = reasoning_id {
                                if store {
                                    // One item reference, at the first part.
                                    reasoning_items.entry(rid).or_insert_with_key(|rid| {
                                        input.push(json!({ "type": "item_reference", "id": rid }));
                                        None
                                    });
                                } else if let Some(&Some(index)) = reasoning_items.get(&rid) {
                                    // Later parts of the same item append their
                                    // summary; the last encrypted content wins.
                                    let item = &mut input[index];
                                    match summary_part {
                                        Some(summary_part) => {
                                            if let Some(summary) =
                                                item["summary"].as_array_mut()
                                            {
                                                summary.push(summary_part);
                                            }
                                        }
                                        None => warnings.push(Warning::Other {
                                            message: format!(
                                                "Cannot append empty reasoning part to existing reasoning sequence. Skipping reasoning part: {}.",
                                                serde_json::to_string(part).unwrap_or_default()
                                            ),
                                        }),
                                    }
                                    if let Some(enc) = encrypted {
                                        item["encrypted_content"] = enc;
                                    }
                                } else {
                                    let mut item = json!({
                                        "type": "reasoning",
                                        "id": rid,
                                        "summary": summary_part.into_iter().collect::<Vec<_>>(),
                                    });
                                    if let Some(enc) = encrypted {
                                        item["encrypted_content"] = enc;
                                    }
                                    reasoning_items.insert(rid, Some(input.len()));
                                    input.push(item);
                                }
                            } else if let Some(enc) = encrypted {
                                input.push(json!({
                                    "type": "reasoning",
                                    "encrypted_content": enc,
                                    "summary": summary_part.into_iter().collect::<Vec<_>>(),
                                }));
                            } else {
                                warnings.push(Warning::Other {
                                    message: format!(
                                        "Non-OpenAI reasoning parts are not supported. Skipping reasoning part: {}.",
                                        serde_json::to_string(part).unwrap_or_default()
                                    ),
                                });
                            }
                        }
                        AssistantPart::ToolResult(part) => {
                            if has_conversation
                                || matches!(part.output, ToolResultOutput::ExecutionDenied { .. })
                                || matches!(&part.output, ToolResultOutput::Json { value, .. } if value.get("type").and_then(Value::as_str) == Some("execution-denied"))
                            {
                                continue;
                            }
                            let id = item_id(ns, &part.provider_options)
                                .unwrap_or_else(|| part.tool_call_id.clone());
                            let kind = provider_tool_kind(tools, &part.tool_name);
                            match kind {
                                Some("shell") => {
                                    if let ToolResultOutput::Json { value, .. } = &part.output
                                        && let Some(item) = provider_tool_result_input("shell", &part.tool_call_id, value)?
                                    { input.push(item); }
                                }
                                _ if store => input.push(json!({"type":"item_reference", "id":id})),
                                Some("tool_search" | "programmatic_tool_calling") => {
                                    if let ToolResultOutput::Json { value, .. } = &part.output {
                                        let kind = kind.unwrap_or_default();
                                        validate_tool_result(kind, value)?;
                                        input.push(if kind == "tool_search" { json!({"type":"tool_search_output", "id":id, "execution":"server", "call_id":null, "status":"completed", "tools":value["tools"]}) } else { json!({"type":"program_output", "id":id, "call_id":part.tool_call_id, "result":value["result"], "status":value["status"]}) });
                                    }
                                }
                                _ => warnings.push(Warning::Other { message:format!("Results for OpenAI tool {} are not sent to the API when store is false", part.tool_name) }),
                            }
                        }
                        AssistantPart::Custom(part) if part.kind == "openai.compaction" => {
                            let id = item_id(ns, &part.provider_options);
                            if has_conversation && id.is_some() {
                                continue;
                            }
                            if let Some(id) = id {
                                if store {
                                    input.push(json!({"type":"item_reference", "id":id}));
                                } else {
                                    input.push(json!({"type":"compaction", "id":id, "encrypted_content":openai_sub_option(ns, &part.provider_options, "encryptedContent")}));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    if let ToolPart::ToolApprovalResponse(approval) = part {
                        if !processed_approval_ids.insert(&approval.approval_id) {
                            continue;
                        }
                        if store && !has_conversation && !has_previous_response_id {
                            input.push(json!({"type":"item_reference", "id":approval.approval_id}));
                        }
                        input.push(json!({"type":"mcp_approval_response", "approval_request_id":approval.approval_id, "approve":approval.approved}));
                        continue;
                    }
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        tool_name,
                        output,
                        provider_options,
                    }) = part
                    else {
                        continue;
                    };
                    if let ToolResultOutput::ExecutionDenied {
                        provider_options, ..
                    } = output
                        && openai_sub_option(ns, provider_options, "approvalId")
                            .and_then(|id| id.as_str().map(|id| !id.is_empty()))
                            .unwrap_or(false)
                    {
                        continue;
                    }
                    let kind = provider_tool_kind(tools, tool_name);
                    if let Some(kind) = kind
                        && kind != "custom"
                        && let ToolResultOutput::Json { value, .. } = output
                        && let Some(item) = provider_tool_result_input(kind, tool_call_id, value)?
                    {
                        input.push(item);
                        continue;
                    }
                    let custom = kind == Some("custom");
                    if !custom
                        && matches!(output, ToolResultOutput::ExecutionDenied { .. })
                        && (programmatic_tool_call_ids.contains(tool_call_id.as_str())
                            || openai_sub_option(ns, provider_options, "caller")
                                .is_some_and(|caller| caller["type"] == "program"))
                    {
                        return Err(AiMuxError::UnsupportedFunctionality(
                            "execution-denied results for programmatic tool calls".into(),
                        ));
                    }
                    let mut content_value =
                        convert_tool_result_output(ns, output, custom, &mut warnings)?;
                    let has_output_schema = tools.is_some_and(|tools| tools.iter().any(|tool| {
                        matches!(tool, Tool::Function(tool) if tool.name == *tool_name && tool.provider_options.as_ref().and_then(|options| options.get("openai")).is_some_and(|options| options.contains_key("outputSchema")))
                    }));
                    if has_output_schema
                        && matches!(
                            output,
                            ToolResultOutput::Text { .. }
                                | ToolResultOutput::ErrorText { .. }
                                | ToolResultOutput::ExecutionDenied { .. }
                        )
                    {
                        content_value = json!(content_value.to_string());
                    }
                    if !matches!(output, ToolResultOutput::Content { .. })
                        && let Some(breakpoint) = scalar_tool_result_cache_breakpoint(ns, output)
                            .or_else(|| {
                                openai_sub_option(ns, provider_options, "promptCacheBreakpoint")
                            })
                    {
                        content_value = json!([{"type":"input_text", "text":content_value, "prompt_cache_breakpoint":breakpoint}]);
                    }
                    let mut item = json!({
                        "type": if custom { "custom_tool_call_output" } else { "function_call_output" },
                        "call_id": tool_call_id,
                        "output": content_value,
                    });
                    if !custom && let Some(caller) = tool_caller(ns, provider_options) {
                        item["caller"] = caller;
                    }
                    input.push(item);
                }
            }
        }
    }

    // When store is false, remove reasoning parts without encrypted content.
    if !store
        && input.iter().any(|item| {
            item.get("type").and_then(|v| v.as_str()) == Some("reasoning")
                && item.get("encrypted_content").is_none()
        })
    {
        warnings.push(Warning::Other {
            message:
                "Reasoning parts without encrypted content are not supported when store is false. Skipping reasoning parts."
                    .to_string(),
        });
        input.retain(|item| {
            item.get("type").and_then(|v| v.as_str()) != Some("reasoning")
                || item.get("encrypted_content").is_some()
        });
    }

    Ok(ResponsesInputResult { input, warnings })
}

/// The `caller` provider option of a tool call or result, on the wire.
fn tool_caller(
    ns: ResponsesNamespace,
    provider_options: &Option<SharedProviderOptions>,
) -> Option<Value> {
    let caller = openai_sub_option(ns, provider_options, "caller")?;
    let mut caller = map_tool_fields(&caller, &[("type", "type"), ("callerId", "caller_id")]);
    if caller["type"] != "program"
        && let Some(object) = caller.as_object_mut()
    {
        object.remove("caller_id");
    }
    Some(caller)
}

fn provider_tool_kind<'a>(tools: Option<&'a [Tool]>, name: &str) -> Option<&'a str> {
    tools?.iter().find_map(|tool| match tool {
        Tool::Provider(tool) if tool.name == name => tool.id.as_str().strip_prefix("openai."),
        _ => None,
    })
}

fn provider_tool_call_input(
    kind: &str,
    name: &str,
    call_id: &str,
    id: Option<&str>,
    args: &Value,
) -> Result<Option<Value>, AiMuxError> {
    let parsed;
    let args = if kind != "custom" && args.is_string() {
        parsed = serde_json::from_str(args.as_str().unwrap_or_default()).map_err(|error| {
            AiMuxError::InvalidArgument {
                message: error.to_string(),
                cause: Some(ErrorCause::source(&error)),
            }
        })?;
        &parsed
    } else {
        args
    };
    if kind != "custom" {
        validate_tool_call(kind, args)?;
    }
    let mut value = match kind {
        "custom" => {
            json!({"type":"custom_tool_call", "call_id":call_id, "name":name, "input":args.as_str().map(str::to_owned).unwrap_or_else(|| args.to_string())})
        }
        "local_shell" => {
            json!({"type":"local_shell_call", "call_id":call_id, "action": map_tool_fields(&args["action"], &[("command", "command"), ("timeoutMs", "timeout_ms"), ("user", "user"), ("workingDirectory", "working_directory"), ("env", "env")])})
        }
        "shell" => {
            json!({"type":"shell_call", "call_id":call_id, "status":"completed", "action":map_tool_fields(&args["action"], &[("commands", "commands"), ("timeoutMs", "timeout_ms"), ("maxOutputLength", "max_output_length")])})
        }
        "apply_patch" => {
            json!({"type":"apply_patch_call", "call_id":args["callId"], "status":"completed", "operation":args["operation"]})
        }
        "computer" => {
            let mut actions = args
                .get("actions")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| AiMuxError::invalid_argument("computer actions"))?;
            for action in &mut actions {
                if action["type"] == "scroll"
                    && let Some(object) = action.as_object_mut()
                {
                    for (key, wire) in [("scrollX", "scroll_x"), ("scrollY", "scroll_y")] {
                        if let Some(v) = object.remove(key) {
                            object.insert(wire.into(), v);
                        }
                    }
                }
            }
            json!({"type":"computer_call", "call_id":call_id, "status":args["status"], "actions":actions, "pending_safety_checks":args["pendingSafetyChecks"]})
        }
        "programmatic_tool_calling" => {
            json!({"type":"program", "call_id":call_id, "code":args["code"], "fingerprint":args["fingerprint"]})
        }
        "tool_search" => {
            let args = if let Some(text) = args.as_str() {
                serde_json::from_str(text).map_err(|error| AiMuxError::InvalidArgument {
                    message: error.to_string(),
                    cause: Some(ErrorCause::source(&error)),
                })?
            } else {
                args.clone()
            };
            let mut value = map_tool_fields(&args, &[("arguments", "arguments")]);
            value["type"] = json!("tool_search_call");
            value["status"] = json!("completed");
            value["execution"] = json!(if args.get("call_id").is_some_and(|id| !id.is_null()) {
                "client"
            } else {
                "server"
            });
            value["call_id"] = args.get("call_id").cloned().unwrap_or(Value::Null);
            value
        }
        _ => return Ok(None),
    };
    if kind == "local_shell" {
        value["action"]["type"] = json!("exec");
    }
    if let Some(id) = id {
        value["id"] = json!(id);
    } else if ["tool_search", "programmatic_tool_calling"].contains(&kind) {
        value["id"] = json!(call_id);
    }
    Ok(Some(value))
}

fn provider_tool_result_input(
    kind: &str,
    call_id: &str,
    result: &Value,
) -> Result<Option<Value>, AiMuxError> {
    if !matches!(
        kind,
        "local_shell" | "shell" | "apply_patch" | "computer" | "tool_search" | "custom"
    ) {
        return Ok(None);
    }
    validate_tool_result(kind, result)?;
    let mut value = match kind {
        "local_shell" => json!({"type":"local_shell_call_output", "output":result["output"]}),
        "shell" => {
            let mut output = result
                .get("output")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| AiMuxError::invalid_argument("shell output"))?;
            for item in &mut output {
                if let Some(outcome) = item.get_mut("outcome").and_then(Value::as_object_mut)
                    && let Some(code) = outcome.remove("exitCode")
                {
                    outcome.insert("exit_code".into(), code);
                }
            }
            json!({"type":"shell_call_output", "output":output})
        }
        "apply_patch" => {
            json!({"type":"apply_patch_call_output", "status":result["status"], "output":result["output"]})
        }
        "computer" => {
            json!({"type":"computer_call_output", "output":map_tool_fields(&result["output"], &[("imageUrl", "image_url"), ("fileId", "file_id"), ("detail", "detail")])})
        }
        "tool_search" => {
            json!({"type":"tool_search_output", "execution":"client", "status":"completed", "tools":result["tools"]})
        }
        "custom" => {
            json!({"type":"custom_tool_call_output", "output":result.as_str().map(str::to_owned).unwrap_or_else(|| result.to_string())})
        }
        _ => return Ok(None),
    };
    if kind == "computer" {
        value["output"]["type"] = json!("computer_screenshot");
        if let Some(checks) = result.get("acknowledgedSafetyChecks") {
            value["acknowledged_safety_checks"] = checks.clone();
        }
    }
    value["call_id"] = json!(call_id);
    Ok(Some(value))
}

fn user_part_options(part: &UserPart) -> &Option<SharedProviderOptions> {
    match part {
        UserPart::Text(part) => &part.provider_options,
        UserPart::File(part) => &part.provider_options,
    }
}

fn system_content(
    ns: ResponsesNamespace,
    text: &str,
    provider_options: &Option<SharedProviderOptions>,
) -> Value {
    match openai_sub_option(ns, provider_options, "promptCacheBreakpoint") {
        Some(value) => {
            json!([{ "type": "input_text", "text": text, "prompt_cache_breakpoint": value }])
        }
        None => json!(text),
    }
}

/// Convert a single user-message content part into the Responses input shape.
fn convert_user_part(
    ns: ResponsesNamespace,
    part: &UserPart,
    index: usize,
) -> Result<Value, AiMuxError> {
    Ok(match part {
        UserPart::Text(part) => json!({ "type": "input_text", "text": part.text }),
        UserPart::File(file) => match &file.data {
            FileData::Data { data } => {
                let b64 = aimux_provider_utils::convert_to_base64(data).into_owned();
                inline_file(
                    ns,
                    &aimux_provider_utils::resolve_full_media_type(file)?,
                    file.filename.as_deref(),
                    &b64,
                    &file.provider_options,
                    index,
                )
            }
            FileData::Reference { reference } => {
                let file_id =
                    aimux_provider_utils::resolve_provider_reference(reference, ns.write_key())?;
                let mut part = json!({ "type": if file.media_type.split('/').next() == Some("image") { "input_image" } else { "input_file" }, "file_id": file_id });
                if part["type"] == "input_image"
                    && let Some(detail) =
                        openai_sub_option(ns, &file.provider_options, "imageDetail")
                {
                    part["detail"] = detail;
                }
                part
            }
            FileData::Url { url, .. } => {
                if file.media_type.split('/').next() == Some("image") {
                    let mut part = json!({ "type": "input_image", "image_url": url });
                    if let Some(detail) =
                        openai_sub_option(ns, &file.provider_options, "imageDetail")
                    {
                        part["detail"] = detail;
                    }
                    part
                } else {
                    json!({ "type": "input_file", "file_url": url })
                }
            }
            FileData::Text { .. } => {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "text file parts".into(),
                ));
            }
        },
    })
}

fn scalar_tool_result_cache_breakpoint(
    ns: ResponsesNamespace,
    output: &ToolResultOutput,
) -> Option<Value> {
    let provider_options = match output {
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
        } => provider_options,
        ToolResultOutput::Content { .. } => return None,
    };
    openai_sub_option(ns, provider_options, "promptCacheBreakpoint")
}

fn convert_tool_result_output(
    ns: ResponsesNamespace,
    output: &ToolResultOutput,
    custom: bool,
    warnings: &mut Vec<Warning>,
) -> Result<Value, AiMuxError> {
    let ToolResultOutput::Content { value } = output else {
        return Ok(aimux_provider_utils::tool_result_to_content(output));
    };
    let mut parts = Vec::new();
    for item in value {
        let (mut converted, provider_options) = match item {
            ToolResultContent::Text(part) => (
                json!({"type":"input_text", "text":part.text}),
                &part.provider_options,
            ),
            ToolResultContent::File(part) => {
                if matches!(part.data, FileData::Text { .. })
                    || (custom && matches!(part.data, FileData::Reference { .. }))
                {
                    warnings.push(Warning::Other {
                        message: format!(
                            "unsupported {}tool content part type: file with data type: {}",
                            if custom { "custom " } else { "" },
                            if matches!(part.data, FileData::Reference { .. }) {
                                "reference"
                            } else {
                                "text"
                            }
                        ),
                    });
                    continue;
                }
                let mut converted = convert_user_part(ns, &UserPart::File(part.clone()), 0)?;
                if converted["type"] == "input_file" && converted.get("file_data").is_some() {
                    converted["filename"] = json!(part.filename.as_deref().unwrap_or("data"));
                }
                if converted["type"] == "input_image"
                    && let Some(detail) =
                        openai_sub_option(ns, &part.provider_options, "imageDetail")
                {
                    converted["detail"] = detail;
                }
                (converted, &part.provider_options)
            }
            ToolResultContent::Custom { .. } => {
                warnings.push(Warning::Other {
                    message: format!(
                        "unsupported {}tool content part type: custom",
                        if custom { "custom " } else { "" }
                    ),
                });
                continue;
            }
        };
        if let Some(breakpoint) = openai_sub_option(ns, provider_options, "promptCacheBreakpoint") {
            converted["prompt_cache_breakpoint"] = breakpoint;
        }
        parts.push(converted);
    }
    Ok(json!(parts))
}

fn inline_file(
    ns: ResponsesNamespace,
    media_type: &str,
    filename: Option<&str>,
    data: &str,
    provider_options: &Option<SharedProviderOptions>,
    index: usize,
) -> Value {
    if media_type.split('/').next() == Some("image") {
        let mut part = json!({ "type": "input_image", "image_url": format!("data:{media_type};base64,{data}") });
        if let Some(detail) = openai_sub_option(ns, provider_options, "imageDetail") {
            part["detail"] = detail;
        }
        part
    } else {
        json!({ "type": "input_file", "filename": filename.map(str::to_owned).unwrap_or_else(|| if media_type == "application/pdf" { format!("part-{index}.pdf") } else { format!("part-{index}") }), "file_data": format!("data:{media_type};base64,{data}") })
    }
}

/// Serialize tool-call arguments as JSON, including null and string values.
fn serialize_arguments(input: &Value) -> String {
    input.to_string()
}

/// Read the `itemId` from a content part's `providerOptions.openai.itemId`.
pub(crate) fn item_id(
    ns: ResponsesNamespace,
    provider_options: &Option<SharedProviderOptions>,
) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| ns.find(v))
        .and_then(|o| o.get("itemId"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
}

/// Read the `phase` from a content part's `providerOptions.openai.phase`.
fn phase_from_provider_options(
    ns: ResponsesNamespace,
    provider_options: &Option<SharedProviderOptions>,
) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| ns.find(v))
        .and_then(|o| o.get("phase"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
}

/// Read the `namespace` from a content part's `providerOptions.openai.namespace`.
fn namespace_from_provider_options(
    ns: ResponsesNamespace,
    provider_options: &Option<SharedProviderOptions>,
) -> Option<String> {
    provider_options
        .as_ref()
        .and_then(|v| ns.find(v))
        .and_then(|o| o.get("namespace"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string)
}

/// Read a sub-key from `providerOptions.openai.<key>` on a content part.
fn openai_sub_option(
    ns: ResponsesNamespace,
    provider_options: &Option<SharedProviderOptions>,
    key: &str,
) -> Option<Value> {
    provider_options
        .as_ref()
        .and_then(|v| ns.find(v))
        .and_then(|o| o.get(key))
        .cloned()
}

/// Apply the host's file-id prefixes (`fileIdPrefixes` in the AI SDK) to a
/// built request body: when a file content part's base64 data starts with one
/// of `prefixes`, it is an uploaded file's id and the part carries a `file_id`
/// field instead of `image_url` / `file_data`. A no-op for no prefixes.
pub(crate) fn apply_file_id_prefixes(body: &mut Value, prefixes: &[&str]) {
    if prefixes.is_empty() {
        return;
    }
    if let Some(input) = body.get_mut("input").and_then(|v| v.as_array_mut()) {
        for msg in input.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                for part in content.iter_mut() {
                    apply_prefix_to_part(part, prefixes);
                }
            }
        }
    }
}

/// Check if a content part's `image_url` or `file_data` contains a data URL
/// whose payload starts with a file-id prefix and, if so, replace it with a
/// `file_id`.
///
/// For `input_file` parts whose media type is an image, the type is also
/// changed to `input_image`, mirroring the AI SDK, which sends image files as
/// `input_image` with a `file_id`.
fn apply_prefix_to_part(part: &mut Value, prefixes: &[&str]) {
    let is_file_id = |data: &str| prefixes.iter().any(|prefix| data.starts_with(prefix));
    // input_image: { type: "input_image", image_url: "data:<mime>;base64,<data>" }
    if part.get("type").and_then(|v| v.as_str()) == Some("input_image") {
        let file_id = part
            .get("image_url")
            .and_then(|v| v.as_str())
            .and_then(extract_base64_data)
            .filter(|data| is_file_id(data))
            .map(std::string::ToString::to_string);
        if let Some(file_id) = file_id
            && let Some(obj) = part.as_object_mut()
        {
            obj.remove("image_url");
            obj.insert("file_id".to_string(), json!(file_id));
        }
    }

    // input_file: { type: "input_file", file_data: "data:<mime>;base64,<data>" }
    if part.get("type").and_then(|v| v.as_str()) == Some("input_file") {
        let file_data = part
            .get("file_data")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        if let Some(file_data) = file_data {
            let media_type = extract_media_type(&file_data).unwrap_or("");
            if let Some(data) = extract_base64_data(&file_data)
                && is_file_id(data)
                && let Some(obj) = part.as_object_mut()
            {
                obj.remove("file_data");
                obj.remove("filename");
                obj.insert("file_id".to_string(), json!(data));
                if media_type.starts_with("image/") {
                    obj.insert("type".to_string(), json!("input_image"));
                }
            }
        }
    }
}

/// Extract the MIME type from a `data:<mime>;base64,<payload>` URL.
fn extract_media_type(data_url: &str) -> Option<&str> {
    let prefix = data_url.strip_prefix("data:")?;
    let end = prefix.find(";base64,")?;
    Some(&prefix[..end])
}

/// Extract the base64 payload from a `data:<mime>;base64,<payload>` URL.
fn extract_base64_data(data_url: &str) -> Option<&str> {
    data_url.split_once(";base64,").map(|(_, rest)| rest)
}
