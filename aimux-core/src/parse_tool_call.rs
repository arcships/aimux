//! Parse and validate provider tool calls at the Core boundary.
//!
//! Rust port of the AI SDK's `parse-tool-call.ts`
//! (`packages/ai/src/generate-text/parse-tool-call.ts`). Providers deliver the
//! model's raw argument text; Core owns JSON parsing, prototype-pollution
//! rejection, schema validation, and the one-shot repair callback.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::tool::{Tool, ToolCall};
use crate::types::ProviderMetadata;

/// Provider-facing tool call before Core parses and validates its input.
///
/// The wire shape matches [`ToolCall`] field for field, except that `input`
/// is the provider's raw argument *text* rather than a parsed value.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RawToolCall {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_executed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ProviderMetadata>,
}

/// Context supplied to a one-shot tool-call repair callback.
#[derive(Debug, Clone)]
pub struct ToolCallRepairContext {
    pub instructions: Option<String>,
    /// Deprecated AI SDK-compatible alias for `instructions`.
    pub system: Option<String>,
    pub messages: Vec<crate::message::ModelMessage>,
    pub tool_call: RawToolCall,
    pub tools: Vec<Tool>,
    pub error: AiMuxError,
}

impl ToolCallRepairContext {
    /// Return the JSON Schema for a named function tool in this repair step.
    ///
    /// Never fails, matching the AI SDK's `inputSchema` repair argument: a
    /// name that does not resolve to a function tool (unknown — the NoSuchTool
    /// repair scenario — or a provider tool, which carries no schema at this
    /// layer) yields the AI SDK's default empty-object schema.
    #[must_use]
    pub fn input_schema(&self, tool_name: &str) -> Value {
        self.tools
            .iter()
            .find_map(|tool| match tool {
                Tool::Function(tool) if tool.name == tool_name => Some(tool.input_schema.clone()),
                _ => None,
            })
            .unwrap_or_else(|| {
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                })
            })
    }
}

type ToolCallRepairFuture =
    Pin<Box<dyn Future<Output = Result<Option<RawToolCall>, AiMuxError>> + Send>>;

/// Async callback that may replace one invalid tool call.
///
/// Core invokes this at most once, and parses and validates the returned call
/// from scratch. Returning `None` keeps the original validation error.
#[derive(Clone)]
pub struct ToolCallRepair(Arc<dyn Fn(ToolCallRepairContext) -> ToolCallRepairFuture + Send + Sync>);

impl ToolCallRepair {
    #[must_use]
    pub fn new<F, Fut>(repair: F) -> Self
    where
        F: Fn(ToolCallRepairContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<RawToolCall>, AiMuxError>> + Send + 'static,
    {
        Self(Arc::new(move |context| Box::pin(repair(context))))
    }

    async fn repair(
        &self,
        context: ToolCallRepairContext,
    ) -> Result<Option<RawToolCall>, AiMuxError> {
        (self.0)(context).await
    }
}

impl std::fmt::Debug for ToolCallRepair {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ToolCallRepair(<callback>)")
    }
}

/// Parse and validate a provider tool call using the AI SDK operation contract.
///
/// JSON is parsed exactly; partial-JSON repair is deliberately not automatic.
/// When a tool set was supplied, lookup, parsing, or schema validation failure
/// gives the optional repair callback one attempt. As in AI SDK, calls made
/// without a tool set bypass repair. A remaining failure is represented on the
/// returned call so callers retain both the model output and its typed error.
pub async fn parse_tool_call(
    tool_call: RawToolCall,
    tools: Option<&[Tool]>,
    repair_tool_call: Option<&ToolCallRepair>,
    messages: &[crate::message::ModelMessage],
    instructions: Option<&str>,
) -> ToolCall {
    let Some(tools) = tools else {
        let parsed = if tool_call.provider_executed == Some(true) && tool_call.dynamic == Some(true)
        {
            parse_json_input(&tool_call)
        } else {
            Err(AiMuxError::NoSuchTool {
                tool_name: tool_call.tool_name.clone(),
                available_tools: None,
                tool_input: Some(tool_call.input.clone()),
            })
        };
        return match parsed {
            Ok(input) => valid_tool_call(tool_call, input, Some(true)),
            Err(error) => invalid_tool_call(tool_call, error),
        };
    };

    match parse_and_validate_tool_call(&tool_call, tools) {
        Ok((input, dynamic)) => valid_tool_call(tool_call, input, dynamic),
        Err(original_error) => {
            let Some(repair_tool_call) = repair_tool_call else {
                return invalid_tool_call(tool_call, original_error);
            };
            let context = ToolCallRepairContext {
                instructions: instructions.map(str::to_owned),
                system: instructions.map(str::to_owned),
                messages: messages.to_vec(),
                tool_call: tool_call.clone(),
                tools: tools.to_vec(),
                error: original_error.clone(),
            };
            let outcome = match repair_tool_call.repair(context).await {
                Ok(Some(repaired)) => RepairOutcome::Repaired(repaired),
                Ok(None) => RepairOutcome::Unchanged,
                Err(repair_error) => RepairOutcome::Failed(repair_error),
            };
            apply_repair_outcome(tool_call, original_error, tools, outcome)
        }
    }
}

/// What a repair attempt produced, independent of how it was obtained (the
/// in-process [`ToolCallRepair`] closure, or a host reply that crossed the
/// bindings as JSON).
enum RepairOutcome {
    Repaired(RawToolCall),
    Unchanged,
    Failed(AiMuxError),
}

/// The AI SDK `parseToolCall` post-repair contract, shared by both paths.
fn apply_repair_outcome(
    tool_call: RawToolCall,
    original_error: AiMuxError,
    tools: &[Tool],
    outcome: RepairOutcome,
) -> ToolCall {
    let cause = match outcome {
        RepairOutcome::Repaired(repaired) => match parse_and_validate_tool_call(&repaired, tools) {
            Ok((input, dynamic)) => return valid_tool_call(repaired, input, dynamic),
            Err(repaired_error) => repaired_error,
        },
        RepairOutcome::Unchanged => return invalid_tool_call(tool_call, original_error),
        RepairOutcome::Failed(repair_error) => repair_error,
    };
    invalid_tool_call(
        tool_call,
        AiMuxError::ToolCallRepair {
            original_error: Box::new(original_error),
            cause: Box::new(cause),
        },
    )
}

fn parse_and_validate_tool_call(
    tool_call: &RawToolCall,
    tools: &[Tool],
) -> Result<(Value, Option<bool>), AiMuxError> {
    let tool = tools.iter().find(|tool| match tool {
        Tool::Function(tool) => tool.name == tool_call.tool_name,
        Tool::Provider(tool) => tool.name == tool_call.tool_name,
    });

    let provider_dynamic =
        tool_call.provider_executed == Some(true) && tool_call.dynamic == Some(true);
    let Some(tool) = tool else {
        if provider_dynamic {
            return parse_json_input(tool_call).map(|input| (input, Some(true)));
        }
        return Err(AiMuxError::NoSuchTool {
            tool_name: tool_call.tool_name.clone(),
            tool_input: Some(tool_call.input.clone()),
            available_tools: Some(
                tools
                    .iter()
                    .map(|tool| match tool {
                        Tool::Function(tool) => tool.name.clone(),
                        Tool::Provider(tool) => tool.name.clone(),
                    })
                    .collect(),
            ),
        });
    };

    let input = parse_json_input(tool_call)?;
    let Tool::Function(function_tool) = tool else {
        return Ok((input, None));
    };
    let validator = jsonschema::validator_for(&function_tool.input_schema).map_err(|error| {
        AiMuxError::InvalidToolInput {
            tool_name: tool_call.tool_name.clone(),
            tool_input: tool_call.input.clone(),
            cause: format!("input schema is invalid: {error}"),
        }
    })?;
    // Cause wording matches the AI SDK's `TypeValidationError` template;
    // `Value` Display is compact JSON, the `JSON.stringify` equivalent.
    validator
        .validate(&input)
        .map_err(|error| AiMuxError::InvalidToolInput {
            tool_name: tool_call.tool_name.clone(),
            tool_input: tool_call.input.clone(),
            cause: format!("Type validation failed: Value: {input}.\nError message: {error}"),
        })?;
    Ok((input, None))
}

fn parse_json_input(tool_call: &RawToolCall) -> Result<Value, AiMuxError> {
    if tool_call.input.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    // Cause wording matches the AI SDK's `JSONParseError` template.
    let value: Value =
        serde_json::from_str(&tool_call.input).map_err(|error| AiMuxError::InvalidToolInput {
            tool_name: tool_call.tool_name.clone(),
            tool_input: tool_call.input.clone(),
            cause: format!(
                "JSON parsing failed: Text: {}.\nError message: {error}",
                tool_call.input
            ),
        })?;
    if contains_forbidden_prototype(&value) {
        return Err(AiMuxError::InvalidToolInput {
            tool_name: tool_call.tool_name.clone(),
            tool_input: tool_call.input.clone(),
            cause: format!(
                "JSON parsing failed: Text: {}.\nError message: Object contains forbidden prototype property",
                tool_call.input
            ),
        });
    }
    Ok(value)
}

// Port of the AI SDK's secure JSON parse (fastify/secure-json-parse): a
// `__proto__` key, or a `constructor` object carrying a `prototype` key,
// anywhere in the tree marks the input invalid. Rust has no prototype
// pollution, but the parsed value crosses the FFI into JS and Python, and
// the valid/invalid classification must match upstream.
fn contains_forbidden_prototype(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            let constructor_prototype = map
                .get("constructor")
                .and_then(Value::as_object)
                .is_some_and(|constructor| constructor.contains_key("prototype"));
            if map.contains_key("__proto__") || constructor_prototype {
                return true;
            }
            map.values().any(contains_forbidden_prototype)
        }
        Value::Array(items) => items.iter().any(contains_forbidden_prototype),
        _ => false,
    }
}

/// Recover the provider's raw argument text: string inputs pass through
/// verbatim (possibly malformed JSON awaiting parse/repair); anything already
/// structured re-serializes.
pub(crate) fn raw_tool_input(input: &Value) -> String {
    match input {
        Value::String(input) => input.clone(),
        input => serde_json::to_string(input).expect("serializing serde_json::Value cannot fail"),
    }
}

fn valid_tool_call(tool_call: RawToolCall, input: Value, dynamic: Option<bool>) -> ToolCall {
    ToolCall {
        tool_call_id: tool_call.tool_call_id,
        tool_name: tool_call.tool_name,
        input,
        provider_executed: tool_call.provider_executed,
        dynamic,
        thought_signature: tool_call.thought_signature,
        provider_metadata: tool_call.provider_metadata,
        invalid: None,
        error: None,
    }
}

fn invalid_tool_call(tool_call: RawToolCall, error: AiMuxError) -> ToolCall {
    // Best effort, for every failure including `NoSuchTool`, exactly as the
    // AI SDK's `parseToolCall` catch-all does: the parsed value when the text
    // is valid JSON, the verbatim text otherwise. Callers replaying an
    // invalid call rely on this — `response_messages` only carries a
    // structured input into the next turn's transcript, so an unknown tool
    // called with perfectly good arguments must still parse here.
    let input = serde_json::from_str(&tool_call.input)
        .unwrap_or_else(|_| Value::String(tool_call.input.clone()));
    ToolCall {
        tool_call_id: tool_call.tool_call_id,
        tool_name: tool_call.tool_name,
        input,
        provider_executed: tool_call.provider_executed,
        dynamic: Some(true),
        thought_signature: tool_call.thought_signature,
        provider_metadata: tool_call.provider_metadata,
        invalid: Some(true),
        error: Some(error),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Host-side tool-call repair (RFC-0035)
// ─────────────────────────────────────────────────────────────────────────────

/// A host's answer to one tool-call repair request.
///
/// The bindings cannot carry a [`ToolCallRepair`] closure, so a host instead
/// reads the invalid call off the result, repairs it in its own language, and
/// hands the answer back as this value. Wire shape:
/// `{"type":"repaired","tool_call":{…}}`, `{"type":"unchanged"}`, or
/// `{"type":"failed","message":"…"}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum ToolCallRepairReply {
    /// Re-validate this replacement call, exactly as the closure path does
    /// for a returned `Some(call)`.
    Repaired { tool_call: RawToolCall },
    /// Keep the original failure — the closure path's `None`.
    Unchanged,
    /// The host's repair attempt itself failed. Reported as
    /// `ToolCallRepair { cause: Other(message) }`, since a host error has no
    /// typed counterpart on this side.
    Failed { message: String },
}

impl From<ToolCallRepairReply> for RepairOutcome {
    fn from(reply: ToolCallRepairReply) -> Self {
        match reply {
            ToolCallRepairReply::Repaired { tool_call } => RepairOutcome::Repaired(tool_call),
            ToolCallRepairReply::Unchanged => RepairOutcome::Unchanged,
            ToolCallRepairReply::Failed { message } => {
                RepairOutcome::Failed(AiMuxError::Other(message))
            }
        }
    }
}

/// The provider's verbatim argument text, when the error kept it.
pub(crate) fn raw_tool_call_text(error: &AiMuxError) -> Option<String> {
    match error {
        AiMuxError::InvalidToolInput { tool_input, .. } => Some(tool_input.clone()),
        AiMuxError::NoSuchTool { tool_input, .. } => tool_input.clone(),
        AiMuxError::ToolCallRepair { original_error, .. } => raw_tool_call_text(original_error),
        _ => None,
    }
}

/// Reconstruct the provider-facing call an invalid [`ToolCall`] was built
/// from.
///
/// The argument text comes from the error, which keeps it verbatim.
/// [`invalid_tool_call`] parses the text into `input` when it is valid JSON,
/// and that loses the JSON string quoting of a bare literal (`"Tokyo"` would
/// come back as `Tokyo`), duplicate keys, number spelling and whitespace, all
/// of which matter when a host re-submits the text unchanged under a new tool
/// name. Only errors from before the text was recorded fall back to
/// [`raw_tool_input`].
///
/// `dynamic` is not recovered: an invalid call always reports `Some(true)`.
/// That only matters for re-deriving the original error, which is read off the
/// call instead.
fn raw_from_invalid(tool_call: &ToolCall) -> Result<(RawToolCall, AiMuxError), AiMuxError> {
    if tool_call.invalid != Some(true) {
        return Err(AiMuxError::InvalidArgument(format!(
            "tool call '{}' is not invalid; nothing to repair",
            tool_call.tool_call_id
        )));
    }
    let error = tool_call.error.clone().ok_or_else(|| {
        AiMuxError::InvalidArgument(format!(
            "invalid tool call '{}' carries no error",
            tool_call.tool_call_id
        ))
    })?;
    Ok((
        RawToolCall {
            tool_call_id: tool_call.tool_call_id.clone(),
            tool_name: tool_call.tool_name.clone(),
            input: raw_tool_call_text(&error).unwrap_or_else(|| raw_tool_input(&tool_call.input)),
            provider_executed: tool_call.provider_executed,
            dynamic: tool_call.dynamic,
            thought_signature: tool_call.thought_signature.clone(),
            provider_metadata: tool_call.provider_metadata.clone(),
        },
        error,
    ))
}

/// The AI SDK's "no tool set, no repair" rule, shared by every entry point
/// that resolves a reply: a call made without tools was never a candidate for
/// repair, so reaching this point with `None` is a caller mistake.
fn required_tools(tools: Option<&[Tool]>) -> Result<&[Tool], AiMuxError> {
    tools.ok_or_else(|| {
        AiMuxError::InvalidArgument(
            "tool calls made without a tool set are not repairable".to_string(),
        )
    })
}

/// Recover the three repair inputs from the same `prompt` and `options` a
/// caller already passed to `generate_text` / `stream_text`.
///
/// Exists so no binding has to re-derive the transcript the model saw: the
/// prompt-to-messages rule and the "instructions travel beside the messages"
/// rule are `generate_text`'s, and stay in one place.
#[must_use]
pub fn tool_call_repair_inputs(
    prompt: impl Into<crate::message::ModelPrompt>,
    options: &crate::generate::GenerateTextOptions,
) -> (
    Vec<crate::message::ModelMessage>,
    Option<&str>,
    Option<&[Tool]>,
) {
    let (messages, instructions) =
        crate::generate::split_prompt(prompt.into(), options.instructions.as_deref());
    (messages, instructions, options.tools.as_deref())
}

/// Build the repair argument a host needs, as JSON, or `None` when the call
/// was made without a tool set.
///
/// Mirrors the AI SDK `repairToolCall` argument: `{tool_call, error,
/// input_schema, tools, messages, instructions}`. `tool_call` is the
/// provider-facing call (raw argument *text*), `input_schema` the schema of
/// the named tool (an empty-object schema when the name does not resolve),
/// and `error` the serialized failure the call already carries.
///
/// `tools` is `Option` for the same reason [`parse_tool_call`]'s is: the AI
/// SDK never repairs a call made without a tool set. That is not a failure —
/// it is the answer "this call is not repairable", and the caller skips it.
///
/// # Errors
///
/// [`AiMuxError::InvalidArgument`] when `tool_call` is not an invalid call, or
/// carries no error.
pub fn tool_call_repair_context(
    tool_call: &ToolCall,
    tools: Option<&[Tool]>,
    messages: &[crate::message::ModelMessage],
    instructions: Option<&str>,
) -> Result<Option<Value>, AiMuxError> {
    let Some(tools) = tools else {
        return Ok(None);
    };
    let (raw, error) = raw_from_invalid(tool_call)?;
    let context = ToolCallRepairContext {
        instructions: instructions.map(str::to_owned),
        system: instructions.map(str::to_owned),
        messages: messages.to_vec(),
        tool_call: raw,
        tools: tools.to_vec(),
        error,
    };
    Ok(Some(serde_json::json!({
        "tool_call": context.tool_call,
        "error": context.error,
        "input_schema": context.input_schema(&context.tool_call.tool_name),
        "tools": context.tools,
        "messages": context.messages,
        "instructions": context.instructions,
    })))
}

/// Resolve one invalid tool call against a host's repair reply.
///
/// Produces the same [`ToolCall`] the in-process [`ToolCallRepair`] closure
/// would have produced for the equivalent return value; both paths share
/// the same post-repair logic.
///
/// # Errors
///
/// [`AiMuxError::InvalidArgument`] when `tools` is `None` (the call was never
/// repairable — [`tool_call_repair_context`] already answered `None` for it),
/// or when `tool_call` is not an invalid call, or carries no error.
pub fn apply_tool_call_repair(
    tool_call: &ToolCall,
    tools: Option<&[Tool]>,
    reply: ToolCallRepairReply,
) -> Result<ToolCall, AiMuxError> {
    let tools = required_tools(tools)?;
    let (raw, error) = raw_from_invalid(tool_call)?;
    Ok(apply_repair_outcome(raw, error, tools, reply.into()))
}

/// Apply a repair reply to a serialized `GenerateTextResult` or
/// `GenerateObjectResult`, or `StreamTextResultAggregated`, returning the
/// patched result.
///
/// Both `tool_calls[]` and the matching `response_messages[]` tool-call part
/// are rewritten, the latter under the same rule `to_response_messages` uses
/// (malformed primitive input is not replayed as a prompt input). A
/// `GenerateObjectResult` is recognised by its nested `raw` result; `object`
/// is untouched, because it is parsed from the model's text, not from a tool
/// call.
///
/// The OpenAI-shaped `ChatCompletion` is deliberately not supported: it drops
/// `invalid` and `error`, so a host cannot tell from it that a call needs
/// repairing in the first place. Repair the native result and convert.
///
/// # Errors
///
/// [`AiMuxError::InvalidArgument`] when `tools` is `None`, when the document
/// has no `tool_calls` array, when `tool_call_id` is missing or ambiguous,
/// when the matched call is not invalid, or when a replacement id conflicts
/// with another call.
pub fn apply_tool_call_repair_to_result(
    result: &Value,
    tools: Option<&[Tool]>,
    tool_call_id: &str,
    reply: ToolCallRepairReply,
) -> Result<Value, AiMuxError> {
    let mut patched = result.clone();
    // `GenerateObjectResult` carries the whole text result under `raw`.
    let target = if patched.get("tool_calls").is_some() {
        &mut patched
    } else {
        patched.get_mut("raw").ok_or_else(|| {
            AiMuxError::InvalidArgument("result: neither tool_calls nor raw".to_string())
        })?
    };

    let calls = target
        .get_mut("tool_calls")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| AiMuxError::InvalidArgument("result: no tool_calls array".to_string()))?;
    // Both `tool_calls` and the transcript are keyed by id here, so a
    // duplicated id cannot be patched unambiguously; refuse rather than
    // rewrite the first entry twice.
    let target_index = calls
        .iter()
        .position(|call| call.get("tool_call_id").and_then(Value::as_str) == Some(tool_call_id))
        .ok_or_else(|| {
            AiMuxError::InvalidArgument(format!("result: no tool call '{tool_call_id}'"))
        })?;
    if calls[target_index + 1..]
        .iter()
        .any(|call| call.get("tool_call_id").and_then(Value::as_str) == Some(tool_call_id))
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "result: tool call id '{tool_call_id}' is not unique"
        )));
    }
    let original: ToolCall = serde_json::from_value(calls[target_index].clone())
        .map_err(|error| AiMuxError::InvalidArgument(format!("result.tool_calls: {error}")))?;

    let repaired = apply_tool_call_repair(&original, tools, reply)?;
    if repaired.tool_call_id != tool_call_id
        && calls.iter().enumerate().any(|(index, call)| {
            index != target_index
                && call.get("tool_call_id").and_then(Value::as_str)
                    == Some(repaired.tool_call_id.as_str())
        })
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "result: repaired tool call id '{}' already exists",
            repaired.tool_call_id
        )));
    }
    calls[target_index] = serde_json::to_value(&repaired)
        .map_err(|error| AiMuxError::InvalidArgument(format!("tool call: {error}")))?;

    // The replayed transcript must agree with `tool_calls`, or the next turn
    // sends the model the unrepaired arguments.
    let replay_input =
        crate::response_messages::response_tool_call_input(&repaired.input, repaired.invalid);
    for part in target
        .get_mut("response_messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get_mut("content")?.as_array_mut())
        .flatten()
    {
        if part.get("type").and_then(Value::as_str) != Some("tool_call")
            || part.get("tool_call_id").and_then(Value::as_str) != Some(tool_call_id)
        {
            continue;
        }
        let Some(part) = part.as_object_mut() else {
            continue;
        };
        // A repair may rename the call; the transcript must keep pointing at
        // the same entry as `tool_calls`.
        part.insert(
            "tool_call_id".to_string(),
            repaired.tool_call_id.clone().into(),
        );
        part.insert("tool_name".to_string(), repaired.tool_name.clone().into());
        part.insert("input".to_string(), replay_input.clone());
    }

    Ok(patched)
}
