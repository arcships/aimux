//! Conversion between `LanguageModelPrompt` and the OpenAI Responses API
//! `input` format, plus request-body construction, tool preparation, usage
//! conversion, and finish-reason mapping.
//!
//! Mirrors the TS sources:
//! - `convert-to-openai-responses-input.ts` -> [`convert_to_responses_input`]
//! - `openai-responses-language-model.ts` `getArgs` ->
//!   [`build_responses_request_body`]
//! - `openai-responses-prepare-tools.ts` -> [`prepare_responses_tools`]
//! - `convert-openai-responses-usage.ts` -> [`convert_responses_usage`]
//! - `map-openai-responses-finish-reason.ts` -> [`map_responses_finish_reason`]

use aimux_core::AiMuxError;
use serde_json::{Value, json};

use aimux_core::language_model_message::{
    AssistantPart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::{FileBytes, FileData, JsonObject, SharedProviderOptions};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, ReasoningEffort, Usage, Warning};

use super::tool_args::{validate_tool_args, validate_tool_call, validate_tool_result};
use super::types::ResponsesUsage;

// -- Model capabilities ------------------------------------------------------
// `GptVersion` / `get_gpt_version` / `get_o_series_version` /
// `ModelCapabilities` / `SystemMessageMode` / `get_model_capabilities` live in
// `crate::openai::convert_common` and are shared with the Chat Completions
// converter (issue M10).
use crate::openai::convert_common::{ModelCapabilities, SystemMessageMode, get_model_capabilities};

/// Compatibility alias: the Responses-specific enum name was merged into the
/// shared [`SystemMessageMode`] during M10. Keeping the alias lets existing
/// import paths (`openai::responses::convert::ResponsesSystemMessageMode`)
/// keep compiling while the module signature uses the shared type.
#[doc(hidden)]
pub use crate::openai::convert_common::SystemMessageMode as ResponsesSystemMessageMode;

// -- Provider options helper -------------------------------------------------

/// The providerOptions keys a Responses model reads, in order of precedence
/// (the first one present wins as a whole), and the key it writes response
/// metadata under. The AI SDK's `OpenAIResponsesLanguageModel` picks
/// `providerOptionsName` from the host: `openai` for the OpenAI API, `azure`
/// for Azure (which still falls back to `openai` when no `azure` options were
/// given).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsesNamespace {
    read: &'static [&'static str],
    write: &'static str,
}

impl ResponsesNamespace {
    /// The OpenAI API: reads and writes `openai`.
    pub const OPENAI: Self = Self::new(&["openai"], "openai");

    /// A host namespace. `read` is non-empty and in order of precedence;
    /// `write` is the metadata key.
    #[must_use]
    pub const fn new(read: &'static [&'static str], write: &'static str) -> Self {
        Self { read, write }
    }

    /// The key response metadata is written under.
    #[must_use]
    pub fn write_key(self) -> &'static str {
        self.write
    }

    /// The first present options object among the read keys.
    fn find(self, provider_options: &SharedProviderOptions) -> Option<&JsonObject> {
        self.read.iter().find_map(|key| provider_options.get(*key))
    }

    /// [`find`](Self::find) for `CallOptions::provider_options`.
    pub(crate) fn find_in(self, provider_options: &SharedProviderOptions) -> Option<&JsonObject> {
        self.read.iter().find_map(|key| provider_options.get(*key))
    }
}

impl Default for ResponsesNamespace {
    fn default() -> Self {
        Self::OPENAI
    }
}

/// How a host differs in the Responses API model: its namespace, and the
/// prefixes that mark a file part's data as an uploaded file id instead of
/// content to inline (`fileIdPrefixes` in the AI SDK).
#[derive(Debug, Clone, Default)]
pub(crate) struct ResponsesProfile {
    pub(crate) namespace: ResponsesNamespace,
    /// Empty: no file part is treated as a file id.
    pub(crate) file_id_prefixes: Vec<&'static str>,
    pub(crate) explicit_message_item_type: bool,
}

/// Get a value from the host namespace's options (`openai.<key>` for OpenAI).
fn openai_option(
    ns: ResponsesNamespace,
    options: &Option<SharedProviderOptions>,
    key: &str,
) -> Option<Value> {
    options
        .as_ref()
        .and_then(|m| ns.find_in(m))
        .and_then(|o| o.get(key))
        .cloned()
}

// -- Input conversion --------------------------------------------------------

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

fn convert_responses_input_with_conversation(
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
                            if provider_executed == &Some(true) {
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
                            if let Some(caller) = openai_sub_option(ns, provider_options, "caller")
                            {
                                let mut caller = map_tool_fields(
                                    &caller,
                                    &[("type", "type"), ("callerId", "caller_id")],
                                );
                                if caller["type"] != "program"
                                    && let Some(object) = caller.as_object_mut()
                                {
                                    object.remove("caller_id");
                                }
                                item["caller"] = caller;
                            }
                            input.push(item);
                        }
                        AssistantPart::Reasoning(ReasoningPart {
                            text,
                            provider_options,
                            ..
                        }) => {
                            let reasoning_id = openai_sub_option(ns, provider_options, "itemId");
                            if (has_previous_response_id || has_conversation)
                                && reasoning_id.is_some()
                            {
                                continue;
                            }
                            if let Some(ref rid) = reasoning_id {
                                if store {
                                    input.push(json!({ "type": "item_reference", "id": rid }));
                                } else {
                                    let encrypted = openai_sub_option(
                                        ns,
                                        provider_options,
                                        "reasoningEncryptedContent",
                                    );
                                    let mut summary: Vec<Value> = Vec::new();
                                    if !text.is_empty() {
                                        summary
                                            .push(json!({ "type": "summary_text", "text": text }));
                                    }
                                    let mut item = json!({
                                        "type": "reasoning",
                                        "id": rid,
                                        "summary": summary,
                                    });
                                    if let Some(enc) = encrypted {
                                        item["encrypted_content"] = json!(enc);
                                    }
                                    input.push(item);
                                }
                            } else {
                                let encrypted = openai_sub_option(
                                    ns,
                                    provider_options,
                                    "reasoningEncryptedContent",
                                );
                                if let Some(enc) = encrypted {
                                    let mut summary: Vec<Value> = Vec::new();
                                    if !text.is_empty() {
                                        summary
                                            .push(json!({ "type": "summary_text", "text": text }));
                                    }
                                    input.push(json!({
                                        "type": "reasoning",
                                        "encrypted_content": enc,
                                        "summary": summary,
                                    }));
                                } else {
                                    warnings.push(Warning::Other {
                                        message: "Non-OpenAI reasoning parts are not supported. Skipping reasoning part.".to_string(),
                                    });
                                }
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
                    input.push(json!({
                        "type": if custom { "custom_tool_call_output" } else { "function_call_output" },
                        "call_id": tool_call_id,
                        "output": content_value,
                    }));
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

fn provider_tool_kind<'a>(tools: Option<&'a [Tool]>, name: &str) -> Option<&'a str> {
    tools?.iter().find_map(|tool| match tool {
        Tool::Provider(tool) if tool.name == name => tool.id.strip_prefix("openai."),
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
        parsed = serde_json::from_str(args.as_str().unwrap_or_default())
            .map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?;
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
                .ok_or_else(|| AiMuxError::InvalidArgument("computer actions".into()))?;
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
                serde_json::from_str(text)
                    .map_err(|error| AiMuxError::InvalidArgument(error.to_string()))?
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
                .ok_or_else(|| AiMuxError::InvalidArgument("shell output".into()))?;
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
                use base64::Engine;
                let b64 = match data {
                    FileBytes::Binary(bytes) => {
                        base64::engine::general_purpose::STANDARD.encode(bytes)
                    }
                    FileBytes::Base64(data) => data.clone(),
                };
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
                let file_id = reference.get(ns.write_key()).ok_or_else(|| {
                    AiMuxError::InvalidArgument(format!("No file reference for {}", ns.write_key()))
                })?;
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
        return Ok(crate::openai::convert::tool_result_to_content(output));
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
fn item_id(
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

// -- Tool preparation --------------------------------------------------------

/// The result of preparing tools for a Responses request body.
#[derive(Debug, Clone)]
pub struct PreparedResponsesTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

/// Prepare the upstream Responses function and provider tool family.
///
/// # Errors
/// Returns an error for invalid provider tool arguments or conflicting namespaces.
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
) -> Result<PreparedResponsesTools, AiMuxError> {
    prepare_responses_tools_for(tools, tool_choice, None, true)
}

fn prepare_responses_tools_for(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
    allowed_tools: Option<&Value>,
    supports_async: bool,
) -> Result<PreparedResponsesTools, AiMuxError> {
    let Some(tools) = tools.as_ref().filter(|tools| !tools.is_empty()) else {
        return Ok(PreparedResponsesTools {
            tools: None,
            tool_choice: None,
            tool_warnings: Vec::new(),
        });
    };
    let mut tool_warnings = Vec::new();
    let mut prepared: Vec<Value> = Vec::new();
    for tool in tools {
        match tool {
            Tool::Function(ft) => {
                let mut value = function_tool_to_json(ft);
                value["parameters"] = crate::openai::convert::normalize_json_schema(
                    &ft.input_schema,
                    &mut tool_warnings,
                )?;
                let opts = ft
                    .provider_options
                    .as_ref()
                    .and_then(|options| options.get("openai"));
                if let Some(opts) = opts {
                    for (key, wire) in [
                        ("async", "async"),
                        ("deferLoading", "defer_loading"),
                        ("allowedCallers", "allowed_callers"),
                    ] {
                        if let Some(v) = opts.get(key) {
                            value[wire] = v.clone();
                        }
                    }
                    if let Some(schema) = opts.get("outputSchema") {
                        value["output_schema"] = crate::openai::convert::normalize_json_schema(
                            schema,
                            &mut tool_warnings,
                        )?;
                    }
                    resolve_async_tool_option(
                        &mut value,
                        supports_async,
                        &ft.name,
                        &mut tool_warnings,
                    );
                    if let Some(namespace) = opts.get("namespace") {
                        let name =
                            namespace
                                .get("name")
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    AiMuxError::InvalidArgument("tool namespace name".into())
                                })?;
                        let description = namespace
                            .get("description")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                AiMuxError::InvalidArgument("tool namespace description".into())
                            })?;
                        if let Some(group) = prepared
                            .iter_mut()
                            .find(|group| group["type"] == "namespace" && group["name"] == name)
                        {
                            if group["description"] != description {
                                return Err(AiMuxError::UnsupportedFunctionality(format!(
                                    "conflicting descriptions for OpenAI tool namespace {name}"
                                )));
                            }
                            if let Some(items) = group["tools"].as_array_mut() {
                                items.push(value);
                            }
                        } else {
                            prepared.push(json!({"type":"namespace", "name":name, "description":description, "tools":[value]}));
                        }
                        continue;
                    }
                }
                prepared.push(value);
            }
            Tool::Provider(tool) => {
                if let Some(mut value) = provider_tool_to_json(tool)? {
                    resolve_async_tool_option(
                        &mut value,
                        supports_async,
                        &tool.name,
                        &mut tool_warnings,
                    );
                    prepared.push(value);
                }
            }
        }
    }
    let mut tool_choice = tool_choice.map(|choice| match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool { tool_name } => {
            let tool = tools.iter().find_map(|tool| match tool {
                Tool::Provider(tool) if tool.name == *tool_name => Some(tool),
                _ => None,
            });
            match tool.map(|tool| tool.id.as_str()) {
                Some("openai.custom") => json!({"type":"custom", "name":tool_name}),
                Some(id)
                    if [
                        "openai.code_interpreter",
                        "openai.file_search",
                        "openai.image_generation",
                        "openai.web_search_preview",
                        "openai.web_search",
                        "openai.mcp",
                        "openai.apply_patch",
                        "openai.computer",
                        "openai.programmatic_tool_calling",
                    ]
                    .contains(&id) =>
                {
                    json!({"type":id.trim_start_matches("openai.")})
                }
                _ => json!({"type":"function", "name":tool_name}),
            }
        }
    });
    if let Some(allowed) = allowed_tools {
        tool_choice = Some(prepare_allowed_tools(allowed, tools, &mut tool_warnings)?);
    }
    Ok(PreparedResponsesTools {
        tools: Some(prepared),
        tool_choice,
        tool_warnings,
    })
}

fn resolve_async_tool_option(
    value: &mut Value,
    supported: bool,
    name: &str,
    warnings: &mut Vec<Warning>,
) {
    if !supported && value.get("async") == Some(&json!(true)) {
        if let Some(object) = value.as_object_mut() {
            object.remove("async");
        }
        warnings.push(Warning::Unsupported {
            feature: format!("async tool calling for {name}"),
            details: Some("Async tool calling is not supported by this model.".into()),
        });
    }
}

fn prepare_allowed_tools(
    allowed: &Value,
    tools: &[Tool],
    warnings: &mut Vec<Warning>,
) -> Result<Value, AiMuxError> {
    let invalid = || AiMuxError::InvalidArgument("allowedTools".into());
    let names = allowed
        .get("toolNames")
        .and_then(Value::as_array)
        .filter(|names| !names.is_empty())
        .ok_or_else(invalid)?;
    let mode = allowed
        .get("mode")
        .map(|mode| {
            mode.as_str()
                .filter(|mode| ["auto", "required"].contains(mode))
                .ok_or_else(invalid)
        })
        .transpose()?
        .unwrap_or("auto");
    let mut entries = Vec::new();
    for name in names {
        let name = name.as_str().ok_or_else(invalid)?;
        let direct = tools.iter().find(|tool| match tool {
            Tool::Function(tool) => tool.name == name,
            Tool::Provider(tool) => tool.name == name,
        });
        let aliases: Vec<_> = tools.iter().filter(|tool| matches!(tool, Tool::Provider(tool) if tool.id.strip_prefix("openai.") == Some(name) && tool.name != name)).collect();
        let tool = if let Some(tool) = direct {
            if !aliases.is_empty() {
                warnings.push(Warning::Unsupported { feature:format!("allowedTools entry {name}"), details:Some("this name is both a tool name and the provider tool name of another tool in this request; the tool with this name is allowed".into()) });
            }
            Some(tool)
        } else {
            if aliases.len() > 1
                && aliases.iter().any(|tool| match tool {
                    Tool::Provider(tool) => tool.id == "openai.custom" || tool.id == "openai.mcp",
                    _ => false,
                })
            {
                warnings.push(Warning::Unsupported { feature:format!("allowedTools entry {name}"), details:Some("several tools share this provider tool name; use the tool name from this request".into()) });
                continue;
            }
            aliases.first().copied()
        };
        let entry = match tool {
            Some(Tool::Function(tool)) => {
                let opts = tool
                    .provider_options
                    .as_ref()
                    .and_then(|options| options.get("openai"));
                if opts.is_some_and(|opts| {
                    opts.contains_key("namespace") || opts.get("deferLoading") == Some(&json!(true))
                }) {
                    warnings.push(Warning::Unsupported { feature:format!("allowedTools entry {name}"), details:Some("namespace and deferred tools are not visible to tool_choice.allowed_tools; the tool is removed from the allowed tools".into()) });
                    continue;
                }
                json!({"type":"function", "name":tool.name})
            }
            Some(Tool::Provider(tool)) => {
                let Some(value) = provider_tool_to_json(tool)? else {
                    entries.push(json!({"type":"function", "name":name}));
                    continue;
                };
                match value["type"].as_str() {
                    Some("custom") => json!({"type":"custom", "name":tool.name}),
                    Some("mcp") => json!({"type":"mcp", "server_label":value["server_label"]}),
                    Some("tool_search") => {
                        warnings.push(Warning::Unsupported { feature:format!("allowedTools entry {name}"), details:Some("tool_search is not visible to tool_choice.allowed_tools; the tool is removed from the allowed tools".into()) });
                        continue;
                    }
                    Some(kind) => json!({"type":kind}),
                    None => continue,
                }
            }
            None => {
                warnings.push(Warning::Unsupported { feature:format!("allowedTools entry {name}"), details:Some("the tool is not part of the tools for this request and is sent as a function tool".into()) });
                json!({"type":"function", "name":name})
            }
        };
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(AiMuxError::UnsupportedFunctionality(
            "allowedTools with only tools that cannot be allow-listed".into(),
        ));
    }
    Ok(json!({"type":"allowed_tools", "mode":mode, "tools":entries}))
}

// Only configured fields are sent: absent optional values are omitted, not null.
fn map_tool_fields(value: &Value, fields: &[(&str, &str)]) -> Value {
    let mut result = json!({});
    for (name, wire) in fields {
        if let Some(value) = value.get(*name) {
            result[*wire] = value.clone();
        }
    }
    result
}

fn provider_tool_to_json(
    tool: &aimux_core::tool::ProviderTool,
) -> Result<Option<Value>, AiMuxError> {
    let kind = tool.id.strip_prefix("openai.").unwrap_or("");
    if ![
        "file_search",
        "local_shell",
        "shell",
        "apply_patch",
        "computer",
        "web_search_preview",
        "web_search",
        "code_interpreter",
        "image_generation",
        "mcp",
        "custom",
        "programmatic_tool_calling",
        "tool_search",
    ]
    .contains(&kind)
    {
        return Ok(None);
    }
    if [
        "local_shell",
        "apply_patch",
        "computer",
        "programmatic_tool_calling",
    ]
    .contains(&kind)
    {
        return Ok(Some(json!({"type":kind})));
    }
    let args = &Value::Object(tool.args.clone());
    validate_tool_args(kind, args)?;
    let fields: &[(&str, &str)] = match kind {
        "file_search" => &[
            ("vectorStoreIds", "vector_store_ids"),
            ("maxNumResults", "max_num_results"),
            ("filters", "filters"),
        ],
        "web_search_preview" | "web_search" => &[
            ("searchContextSize", "search_context_size"),
            ("userLocation", "user_location"),
        ],
        "image_generation" => &[
            ("action", "action"),
            ("background", "background"),
            ("inputFidelity", "input_fidelity"),
            ("model", "model"),
            ("moderation", "moderation"),
            ("partialImages", "partial_images"),
            ("quality", "quality"),
            ("outputCompression", "output_compression"),
            ("outputFormat", "output_format"),
            ("size", "size"),
        ],
        "mcp" => &[
            ("serverLabel", "server_label"),
            ("authorization", "authorization"),
            ("connectorId", "connector_id"),
            ("headers", "headers"),
            ("serverDescription", "server_description"),
            ("serverUrl", "server_url"),
        ],
        "custom" => &[
            ("description", "description"),
            ("format", "format"),
            ("async", "async"),
        ],
        "tool_search" => &[
            ("execution", "execution"),
            ("description", "description"),
            ("parameters", "parameters"),
        ],
        _ => &[],
    };
    let mut value = map_tool_fields(args, fields);
    value["type"] = json!(kind);
    if let Some(location) = args.get("userLocation") {
        value["user_location"] = map_tool_fields(
            location,
            &[
                ("type", "type"),
                ("country", "country"),
                ("city", "city"),
                ("region", "region"),
                ("timezone", "timezone"),
            ],
        );
    }
    match kind {
        "file_search" => {
            if let Some(ranking) = args.get("ranking") {
                value["ranking_options"] = map_tool_fields(
                    ranking,
                    &[("ranker", "ranker"), ("scoreThreshold", "score_threshold")],
                );
            }
        }
        "web_search" => {
            if let Some(filters) = args.get("filters") {
                value["filters"] = map_tool_fields(
                    filters,
                    &[
                        ("allowedDomains", "allowed_domains"),
                        ("blockedDomains", "blocked_domains"),
                    ],
                );
            }
            if let Some(access) = args.get("externalWebAccess") {
                value["external_web_access"] = access.clone();
            }
        }
        "code_interpreter" => {
            value["container"] = match args.get("container") {
                Some(container) if container.is_string() => container.clone(),
                container => {
                    let mut value = container
                        .map(|container| map_tool_fields(container, &[("fileIds", "file_ids")]))
                        .unwrap_or_else(|| json!({}));
                    value["type"] = json!("auto");
                    value
                }
            }
        }
        "image_generation" => {
            if let Some(mask) = args.get("inputImageMask") {
                value["input_image_mask"] =
                    map_tool_fields(mask, &[("fileId", "file_id"), ("imageUrl", "image_url")]);
            }
        }
        "custom" => {
            value["name"] = json!(tool.name);
            if let Some(format) = args.get("format") {
                value["format"] = map_tool_fields(
                    format,
                    &[
                        ("type", "type"),
                        ("syntax", "syntax"),
                        ("definition", "definition"),
                    ],
                );
            }
        }
        "mcp" => {
            if let Some(allowed) = args.get("allowedTools") {
                value["allowed_tools"] = if allowed.is_array() {
                    allowed.clone()
                } else {
                    map_tool_fields(
                        allowed,
                        &[("readOnly", "read_only"), ("toolNames", "tool_names")],
                    )
                };
            }
            value["require_approval"] = args
                .get("requireApproval")
                .map(|approval| {
                    if approval.is_string() {
                        approval.clone()
                    } else if let Some(never) = approval.get("never") {
                        json!({"never":map_tool_fields(never, &[("toolNames", "tool_names")])})
                    } else {
                        json!("never")
                    }
                })
                .unwrap_or_else(|| json!("never"));
        }
        "shell" => {
            if let Some(environment) = args.get("environment") {
                value["environment"] = map_shell_environment(environment)?;
            }
        }
        _ => {}
    }
    Ok(Some(value))
}

fn map_shell_environment(environment: &Value) -> Result<Value, AiMuxError> {
    let kind = environment
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("local");
    let mut value = match kind {
        "containerReference" => map_tool_fields(environment, &[("containerId", "container_id")]),
        "containerAuto" => map_tool_fields(
            environment,
            &[("fileIds", "file_ids"), ("memoryLimit", "memory_limit")],
        ),
        _ => map_tool_fields(environment, &[("skills", "skills")]),
    };
    value["type"] = json!(match kind {
        "containerReference" => "container_reference",
        "containerAuto" => "container_auto",
        _ => "local",
    });
    if kind == "containerAuto" {
        if let Some(policy) = environment.get("networkPolicy") {
            value["network_policy"] = map_tool_fields(
                policy,
                &[
                    ("type", "type"),
                    ("allowedDomains", "allowed_domains"),
                    ("domainSecrets", "domain_secrets"),
                ],
            );
        }
        if let Some(skills) = environment.get("skills").and_then(Value::as_array) {
            value["skills"] = Value::Array(skills.iter().map(|skill| {
                if skill["type"] == "skillReference" {
                    let id = skill.get("providerReference").and_then(|reference| reference.get("openai")).and_then(Value::as_str).ok_or_else(|| AiMuxError::InvalidArgument("No skill reference for openai".into()))?;
                    Ok(json!({"type":"skill_reference", "skill_id":id, "version":skill.get("version").cloned().unwrap_or_else(|| json!("latest"))}))
                } else {
                    let mut value = map_tool_fields(skill, &[("name", "name"), ("description", "description")]);
                    value["type"] = json!("inline");
                    value["source"] = map_tool_fields(&skill["source"], &[("type", "type"), ("mediaType", "media_type"), ("data", "data")]);
                    Ok(value)
                }
            }).collect::<Result<Vec<_>, AiMuxError>>()?);
        }
    }
    Ok(value)
}

/// Convert a `FunctionTool` into the Responses `function` tool JSON shape.
fn function_tool_to_json(t: &FunctionTool) -> Value {
    let mut func = json!({
        "type": "function",
        "name": t.name,
        "parameters": t.input_schema,
    });
    if let Some(ref desc) = t.description {
        func["description"] = json!(desc);
    }
    func["strict"] = json!(t.strict.unwrap_or(false));
    func
}

// -- Request body ------------------------------------------------------------

/// Result of building a Responses request body.
pub struct ResponsesRequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}
/// Compatibility warnings for call options the Responses API does not carry.
fn push_unsupported_call_option_warnings(options: &CallOptions, warnings: &mut Vec<Warning>) {
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }
    if options.seed.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "seed".to_string(),
            details: None,
        });
    }
    if options.presence_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "presencePenalty".to_string(),
            details: None,
        });
    }
    if options.frequency_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "frequencyPenalty".to_string(),
            details: None,
        });
    }
    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".to_string(),
            details: None,
        });
    }
}

/// Resolve the Responses reasoning config: `reasoningEffort` (provider option
/// wins over top-level `reasoning`), `reasoningSummary` (defaults to "detailed"
/// when an effort other than "none" applies), and whether the model reasons.
fn resolve_responses_reasoning(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    options: &CallOptions,
    caps: &ModelCapabilities,
) -> (Option<String>, Option<String>, bool) {
    let resolved_reasoning_effort: Option<String> =
        openai_option(ns, provider_opts, "reasoningEffort")
            .map(|v| {
                v.as_str()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .or_else(|| {
                if options.reasoning.is_some_and(ReasoningEffort::is_custom) {
                    options.reasoning.map(|r| r.to_string())
                } else {
                    None
                }
            });

    let resolved_reasoning_summary = match openai_option(ns, provider_opts, "reasoningSummary") {
        Some(value) => value.as_str().map(str::to_owned),
        None => resolved_reasoning_effort
            .as_deref()
            .filter(|effort| *effort != "none")
            .map(|_| "detailed".to_string()),
    };

    let is_reasoning_model = openai_option(ns, provider_opts, "forceReasoning")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(caps.is_reasoning_model);

    (
        resolved_reasoning_effort,
        resolved_reasoning_summary,
        is_reasoning_model,
    )
}

/// Warn when `conversation` and `previousResponseId` are both set.
fn warn_conversation_conflict(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    warnings: &mut Vec<Warning>,
) {
    let has_conversation = openai_option(ns, provider_opts, "conversation").is_some();
    let has_previous_response_id = openai_option(ns, provider_opts, "previousResponseId").is_some();
    if has_conversation && has_previous_response_id {
        warnings.push(Warning::Unsupported {
            feature: "conversation".to_string(),
            details: Some(
                "conversation and previousResponseId cannot be used together".to_string(),
            ),
        });
    }
}

fn resolve_responses_system_message_mode(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    caps: &ModelCapabilities,
) -> SystemMessageMode {
    openai_option(ns, provider_opts, "systemMessageMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
        .map(|s| match s.as_str() {
            "developer" => SystemMessageMode::Developer,
            "remove" => SystemMessageMode::Remove,
            _ => SystemMessageMode::System,
        })
        .unwrap_or(if is_reasoning_model {
            SystemMessageMode::Developer
        } else {
            caps.system_message_mode
        })
}

/// temperature / top_p, subject to reasoning-model restrictions.
fn apply_responses_sampling(
    ns: ResponsesNamespace,
    body: &mut Value,
    options: &CallOptions,
    caps: &ModelCapabilities,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    warnings: &mut Vec<Warning>,
) {
    let provider_opts = &options.provider_options;
    let mut temperature = options.temperature;
    let mut top_p = options.top_p;

    if is_reasoning_model {
        let allow_non_reasoning = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;
        if !allow_non_reasoning {
            if temperature.is_some() {
                temperature = None;
                warnings.push(Warning::Unsupported {
                    feature: "temperature".to_string(),
                    details: Some("temperature is not supported for reasoning models".to_string()),
                });
            }
            if top_p.is_some() {
                top_p = None;
                warnings.push(Warning::Unsupported {
                    feature: "topP".to_string(),
                    details: Some("topP is not supported for reasoning models".to_string()),
                });
            }
        }
    } else {
        for key in [
            "reasoningEffort",
            "reasoningSummary",
            "reasoningMode",
            "reasoningContext",
        ] {
            if openai_option(ns, provider_opts, key).is_some() {
                warnings.push(Warning::Unsupported {
                    feature: key.to_string(),
                    details: Some(format!("{key} is not supported for non-reasoning models")),
                });
            }
        }
    }

    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
}

/// `text.format` (json_schema / json_object) plus `verbosity`.
fn apply_responses_text_format(
    ns: ResponsesNamespace,
    body: &mut Value,
    options: &CallOptions,
    provider_opts: &Option<SharedProviderOptions>,
    warnings: &mut Vec<Warning>,
) {
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description,
            } => {
                let mut text = json!({});
                match schema {
                    Some(schema) => {
                        let strict_json = openai_option(ns, provider_opts, "strictJsonSchema")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        text["format"] = json!({
                            "type": "json_schema",
                            "strict": strict_json,
                            "name": name.clone().unwrap_or_else(|| "response".to_string()),
                            "schema": crate::openai::convert::normalize_json_schema(schema, warnings).unwrap_or_else(|_| schema.clone()),
                        });
                        if let Some(description) = description {
                            text["format"]["description"] = json!(description);
                        }
                    }
                    None => {
                        text["format"] = json!({ "type": "json_object" });
                    }
                }
                body["text"] = text;
            }
        }
    }

    if let Some(verbosity) = openai_option(ns, provider_opts, "textVerbosity") {
        let text = body.get_mut("text").and_then(|t| t.as_object_mut());
        match text {
            Some(obj) => {
                obj.insert("verbosity".to_string(), verbosity);
            }
            None => {
                body["text"] = json!({ "verbosity": verbosity });
            }
        }
    }
}

/// The computed `include` list (store=false on reasoning models adds
/// `reasoning.encrypted_content`).
fn resolve_responses_include(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    tools: Option<&[Tool]>,
) -> Option<Vec<Value>> {
    let mut include: Option<Vec<Value>> =
        openai_option(ns, provider_opts, "include").and_then(|v| v.as_array().cloned());

    let add_include = |key: &str, inc: &mut Option<Vec<Value>>| {
        let already = inc
            .as_ref()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(key)));
        if !already {
            match inc {
                Some(arr) => arr.push(json!(key)),
                None => *inc = Some(vec![json!(key)]),
            }
        }
    };

    // store defaults to true; only the explicit `false` triggers encrypted_content.
    let store_explicit = openai_option(ns, provider_opts, "store").and_then(|v| v.as_bool());
    if store_explicit == Some(false) && is_reasoning_model {
        add_include("reasoning.encrypted_content", &mut include);
    }

    let top_logprobs = openai_option(ns, provider_opts, "logprobs")
        .and_then(|v| if v == true { Some(20) } else { v.as_u64() });
    if top_logprobs.is_some_and(|value| value > 0) {
        add_include("message.output_text.logprobs", &mut include);
    }
    for tool in tools.into_iter().flatten() {
        if let Tool::Provider(tool) = tool {
            match tool.id.as_str() {
                "openai.web_search" | "openai.web_search_preview"
                    if openai_option(ns, provider_opts, "includeWebSearchSources")
                        != Some(json!(false)) =>
                {
                    add_include("web_search_call.action.sources", &mut include)
                }
                "openai.code_interpreter" => {
                    add_include("code_interpreter_call.outputs", &mut include)
                }
                _ => {}
            }
        }
    }
    include
}

/// Pass-through of the remaining Responses provider options (only sent when
/// set).
fn apply_responses_provider_options(
    ns: ResponsesNamespace,
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
) {
    let mut set = |key: &str, body_key: &str| {
        if let Some(v) = openai_option(ns, provider_opts, key) {
            body[body_key] = v;
        }
    };
    set("conversation", "conversation");
    set("maxToolCalls", "max_tool_calls");
    set("metadata", "metadata");
    set("parallelToolCalls", "parallel_tool_calls");
    set("previousResponseId", "previous_response_id");
    set("user", "user");
    set("instructions", "instructions");
    set("promptCacheKey", "prompt_cache_key");
    set("promptCacheOptions", "prompt_cache_options");
    set("promptCacheRetention", "prompt_cache_retention");
    set("safetyIdentifier", "safety_identifier");
    set("truncation", "truncation");
    if let Some(value) = openai_option(ns, provider_opts, "logprobs").and_then(|v| {
        if v == true {
            Some(json!(20))
        } else {
            v.as_u64().map(|n| json!(n))
        }
    }) {
        body["top_logprobs"] = value;
    }
    if let Some(values) =
        openai_option(ns, provider_opts, "contextManagement").and_then(|v| v.as_array().cloned())
    {
        body["context_management"] = json!(
            values
                .into_iter()
                .map(|v| {
                    let mut entry = json!({ "type": v["type"] });
                    if let Some(threshold) = v.get("compactThreshold") {
                        entry["compact_threshold"] = threshold.clone();
                    }
                    entry
                })
                .collect::<Vec<_>>()
        );
    }
}

/// `service_tier` with model-capability validation.
fn apply_responses_service_tier(
    ns: ResponsesNamespace,
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) {
    if let Some(st) = openai_option(ns, provider_opts, "serviceTier")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
    {
        match st.as_str() {
            "flex" if !caps.supports_flex_processing => {
                warnings.push(Warning::Unsupported {
                    feature: "serviceTier".to_string(),
                    details: Some(
                        "flex processing is only available for o3, o4-mini, and gpt-5 models"
                            .to_string(),
                    ),
                });
            }
            "priority" | "fast" if !caps.supports_priority_processing => {
                warnings.push(Warning::Unsupported {
                    feature: "serviceTier".to_string(),
                    details: Some("priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported".to_string()),
                });
            }
            _ => {
                body["service_tier"] = json!(st);
            }
        }
    }
}

/// `reasoning` block for reasoning models.
fn apply_responses_reasoning_block(
    ns: ResponsesNamespace,
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    resolved_reasoning_summary: &Option<String>,
) {
    if !is_reasoning_model {
        return;
    }
    let effort = resolved_reasoning_effort.as_ref();
    let summary = resolved_reasoning_summary.as_ref();
    let mode = openai_option(ns, provider_opts, "reasoningMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));
    let context = openai_option(ns, provider_opts, "reasoningContext")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));

    if effort.is_some() || summary.is_some() || mode.is_some() || context.is_some() {
        let mut reasoning = json!({});
        if let Some(e) = effort {
            reasoning["effort"] = json!(e);
        }
        if let Some(s) = summary {
            reasoning["summary"] = json!(s);
        }
        if let Some(m) = mode {
            reasoning["mode"] = json!(m);
        }
        if let Some(c) = context {
            reasoning["context"] = json!(c);
        }
        body["reasoning"] = reasoning;
    }
}

/// Build the OpenAI Responses API request body and warnings.
///
/// # Errors
/// Rejects unsupported prompt parts and invalid tool arguments.
pub fn build_responses_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<ResponsesRequestBodyResult, AiMuxError> {
    build_responses_request_body_for(ResponsesNamespace::OPENAI, model_id, options, stream)
}

/// [`build_responses_request_body`] for a host with its own providerOptions
/// namespace.
///
/// # Errors
/// Rejects unsupported prompt parts and invalid tool arguments.
pub fn build_responses_request_body_for(
    ns: ResponsesNamespace,
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<ResponsesRequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);
    let provider_opts = &options.provider_options;

    // -- Warnings for unsupported call options --
    push_unsupported_call_option_warnings(options, &mut warnings);

    // -- Reasoning resolution --
    let (mut resolved_reasoning_effort, mut resolved_reasoning_summary, is_reasoning_model) =
        resolve_responses_reasoning(ns, provider_opts, options, &caps);
    if let Some(efforts) = caps.supported_reasoning_efforts
        && resolved_reasoning_effort
            .as_deref()
            .is_some_and(|effort| !efforts.contains(&effort))
    {
        warnings.push(Warning::Unsupported {
            feature: "reasoningEffort".to_string(),
            details: Some(format!(
                "{model_id} only supports the following reasoning efforts: {}",
                efforts.join(", ")
            )),
        });
        resolved_reasoning_effort = None;
        resolved_reasoning_summary = openai_option(ns, provider_opts, "reasoningSummary")
            .and_then(|v| v.as_str().map(str::to_owned));
    }

    // -- conversation + previousResponseId conflict --
    warn_conversation_conflict(ns, provider_opts, &mut warnings);

    // -- System message mode --
    let system_message_mode =
        resolve_responses_system_message_mode(ns, provider_opts, is_reasoning_model, &caps);

    // -- Input conversion --
    let store_bool = openai_option(ns, provider_opts, "store")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let has_previous_response_id = openai_option(ns, provider_opts, "previousResponseId").is_some();
    let mut input_result = convert_responses_input_with_conversation(
        ns,
        &options.prompt,
        system_message_mode,
        store_bool,
        has_previous_response_id,
        openai_option(ns, provider_opts, "conversation").is_some(),
        options.tools.as_deref(),
    )?;
    if let Some(effort) = openai_option(ns, provider_opts, "reasoningEffortUpdate")
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        let reason = if !caps.supports_configuration_update {
            Some("reasoningEffortUpdate is only supported by GPT-6 and later models".to_string())
        } else if openai_option(ns, provider_opts, "reasoningMode") == Some(json!("pro"))
            || openai_option(ns, provider_opts, "contextManagement").is_some()
            || openai_option(ns, provider_opts, "truncation") == Some(json!("auto"))
        {
            Some("reasoningEffortUpdate requires standard reasoning mode without automatic compaction or automatic truncation".to_string())
        } else if caps
            .supported_reasoning_efforts
            .is_some_and(|values| !values.contains(&effort.as_str()))
        {
            Some(format!(
                "{model_id} does not support reasoning effort {effort}"
            ))
        } else {
            None
        };
        if let Some(details) = reason {
            warnings.push(Warning::Unsupported {
                feature: "reasoningEffortUpdate".to_string(),
                details: Some(details),
            });
        } else if input_result.input.first().is_none_or(|item| {
            item["type"] != "configuration_update" || item["reasoning"]["effort"] != effort
        }) {
            input_result.input.insert(
                0,
                json!({ "type": "configuration_update", "reasoning": { "effort": effort } }),
            );
        }
    }
    if openai_option(ns, provider_opts, "compactionTrigger") == Some(json!(true)) {
        input_result
            .input
            .push(json!({ "type": "compaction_trigger" }));
    }
    warnings.extend(input_result.warnings);

    // -- Base body --
    let mut body = json!({
        "model": model_id,
        "input": input_result.input,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    // temperature / top_p (subject to reasoning-model restrictions)
    apply_responses_sampling(
        ns,
        &mut body,
        options,
        &caps,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &mut warnings,
    );

    // -- Response format (text.format) + verbosity --
    apply_responses_text_format(ns, &mut body, options, provider_opts, &mut warnings);

    // -- include (computed) --
    let include = resolve_responses_include(
        ns,
        provider_opts,
        is_reasoning_model,
        options.tools.as_deref(),
    );
    if let Some(inc) = include {
        body["include"] = json!(inc);
    }

    // -- store (only sent when explicitly set) --
    if let Some(s) = openai_option(ns, provider_opts, "store").and_then(|v| v.as_bool()) {
        body["store"] = json!(s);
    }

    // -- Other provider options (only sent when set) --
    apply_responses_provider_options(ns, &mut body, provider_opts);
    if caps.supports_configuration_update && body.get("prompt_cache_retention").is_some() {
        body.as_object_mut()
            .expect("request body is an object")
            .remove("prompt_cache_retention");
        warnings.push(Warning::Unsupported {
            feature: "promptCacheRetention".to_string(),
            details: Some("promptCacheRetention is not supported by GPT-6 and later models; use promptCacheOptions instead".to_string()),
        });
    }

    if is_reasoning_model
        && caps.supported_reasoning_efforts.is_some()
        && !(resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters)
    {
        let mut removed = body
            .as_object_mut()
            .expect("request body is an object")
            .remove("top_logprobs")
            .is_some();
        if let Some(include) = body.get_mut("include").and_then(Value::as_array_mut) {
            let len = include.len();
            include.retain(|value| value.as_str() != Some("message.output_text.logprobs"));
            removed |= include.len() != len;
        }
        if body
            .get("include")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            body.as_object_mut()
                .expect("request body is an object")
                .remove("include");
        }
        if removed {
            warnings.push(Warning::Unsupported {
                feature: "logprobs".to_string(),
                details: Some("logprobs is not supported for reasoning models".to_string()),
            });
        }
    }

    // -- service_tier (with capability validation) --
    apply_responses_service_tier(ns, &mut body, provider_opts, &caps, &mut warnings);

    // -- reasoning block (reasoning models only) --
    apply_responses_reasoning_block(
        ns,
        &mut body,
        provider_opts,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &resolved_reasoning_summary,
    );

    // -- Tools --
    let allowed_tools = openai_option(ns, provider_opts, "allowedTools");
    let prepared = prepare_responses_tools_for(
        &options.tools,
        options.tool_choice.as_ref(),
        allowed_tools.as_ref(),
        caps.supports_configuration_update,
    )?;
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }
    for tw in prepared.tool_warnings {
        warnings.push(tw);
    }

    Ok(ResponsesRequestBodyResult { body, warnings })
}

// -- File id prefixes --------------------------------------------------------

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

// -- Usage conversion --------------------------------------------------------

/// Convert a Responses API usage into the core `Usage` type.
///
/// Mirrors TS `convertOpenAIResponsesUsage`:
/// - `input.total = input_tokens`
/// - `input.noCache = input_tokens - cached_tokens - cache_write_tokens`
/// - `input.cacheRead = cached_tokens`
/// - `input.cacheWrite = cache_write_tokens`
/// - `output.total = output_tokens`
/// - `output.text = output_tokens - reasoning_tokens`
/// - `output.reasoning = reasoning_tokens`
///
/// `raw` is the untouched wire `usage` object; it is stored on `Usage.raw`
/// (RFC-0015 P0-3). Re-serializing the typed struct instead would drop the
/// `orchestration_*` counters and add explicit nulls, so the caller passes the
/// original value through — matching the TS `raw: usage`.
#[must_use]
pub fn convert_responses_usage(usage: Option<&ResponsesUsage>, raw: Option<Value>) -> Usage {
    let Some(usage) = usage else {
        return Usage {
            raw: raw.and_then(|value| value.as_object().cloned()),
            ..Default::default()
        };
    };

    let input_tokens = usage.input_tokens;
    let output_tokens = usage.output_tokens;

    let cached_tokens = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let cache_write = usage
        .input_tokens_details
        .as_ref()
        .and_then(|d| d.cache_write_tokens);
    let reasoning_tokens = usage
        .output_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    let no_cache = input_tokens - cached_tokens - cache_write.unwrap_or(0);
    let text_tokens = output_tokens - reasoning_tokens;

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(input_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached_tokens),
            cache_write,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(output_tokens),
            text: Some(text_tokens),
            reasoning: Some(reasoning_tokens),
        },
        raw: raw.and_then(|value| value.as_object().cloned()),
    }
}

/// Helper: build a `ResponsesUsage` from a raw JSON `usage` object.
#[must_use]
pub fn parse_usage(raw: &Value) -> Option<ResponsesUsage> {
    serde_json::from_value(raw.clone()).ok()
}

// -- Finish reason -----------------------------------------------------------

/// Map a Responses API finish reason into the unified `FinishReason`.
///
/// Mirrors TS `mapOpenAIResponseFinishReason`. When `finish_reason` is
/// `None`/null, the unified reason is `tool-calls` if there were function
/// calls, else `stop`. `"max_output_tokens"` -> `length`,
/// `"content_filter"` -> `content-filter`; otherwise `tool-calls` if there
/// were function calls, else `other`.
#[must_use]
pub fn map_responses_finish_reason(
    finish_reason: Option<&str>,
    has_function_call: bool,
) -> FinishReason {
    let unified = match finish_reason {
        None => {
            if has_function_call {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        Some("max_output_tokens") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some(_) => {
            if has_function_call {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Other
            }
        }
    };
    FinishReason {
        unified,
        raw: finish_reason.map(std::string::ToString::to_string),
    }
}
