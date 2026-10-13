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

use crate::content::ContentPart;
use crate::error::AiMuxError;
use crate::result::GenerateContent;
use crate::tool::{RawToolCall, Tool, ToolCall};

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
            Ok(input) => parsed_tool_call(tool_call, input, Some(true)),
            Err(error) => invalid_tool_call(tool_call, error),
        };
    };

    match parse_and_validate_tool_call(&tool_call, tools) {
        Ok((input, dynamic)) => parsed_tool_call(tool_call, input, dynamic),
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
            Ok((input, dynamic)) => return parsed_tool_call(repaired, input, dynamic),
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
pub(crate) fn contains_forbidden_prototype(value: &Value) -> bool {
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

fn parsed_tool_call(tool_call: RawToolCall, input: Value, dynamic: Option<bool>) -> ToolCall {
    ToolCall {
        tool_call_id: tool_call.tool_call_id,
        tool_name: tool_call.tool_name,
        input,
        provider_executed: tool_call.provider_executed,
        dynamic,
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
        invalid: Some(true),
        error: Some(error),
        ..parsed_tool_call(tool_call, input, Some(true))
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
/// name. Only errors from before the text was recorded fall back to the
/// compact JSON of `input`.
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
            // An invalid call keeps unparsable argument text as a JSON string
            // (see `invalid_tool_call`); that text is the raw input.
            input: raw_tool_call_text(&error).unwrap_or_else(|| match &tool_call.input {
                Value::String(text) => text.clone(),
                input => input.to_string(),
            }),
            provider_executed: tool_call.provider_executed,
            dynamic: tool_call.dynamic,
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

/// Apply a repair reply to a serialized `GenerateTextResult` (from
/// `generate_text` or a consumed `stream_text`), returning the patched result.
///
/// The steps are the result's facts, so the repair is applied there: the
/// matching tool-call content part, the call embedded in a matching approval
/// request, and the tool-call part of the step's response messages, the
/// latter under the same rule `to_response_messages` uses (malformed
/// primitive input is not replayed as a prompt input). A renamed call also
/// renames the response messages' approval request. The derived values of
/// the returned document (`tool_calls`, `content`, `response_messages`, ...)
/// are written from the patched steps.
///
/// The OpenAI-shaped `ChatCompletion` is deliberately not supported: it drops
/// `invalid` and `error`, so a host cannot tell from it that a call needs
/// repairing in the first place. Repair the native result and convert.
///
/// # Errors
///
/// [`AiMuxError::InvalidArgument`] when `tools` is `None`, when the document
/// is not a text result, when `tool_call_id` is missing or ambiguous, when
/// the matched call is not invalid, or when a replacement id conflicts with
/// another call.
pub fn apply_tool_call_repair_to_result(
    result: &Value,
    tools: Option<&[Tool]>,
    tool_call_id: &str,
    reply: ToolCallRepairReply,
) -> Result<Value, AiMuxError> {
    let mut result: crate::generate::GenerateTextResult = serde_json::from_value(result.clone())
        .map_err(|error| AiMuxError::InvalidArgument(format!("result: {error}")))?;

    // Calls and the transcript are keyed by id here, so a duplicated id
    // cannot be patched unambiguously; refuse rather than rewrite the first
    // entry twice.
    let calls = result.tool_calls();
    let mut matching = calls
        .iter()
        .filter(|call| call.tool_call_id == tool_call_id);
    let original = (*matching.next().ok_or_else(|| {
        AiMuxError::InvalidArgument(format!("result: no tool call '{tool_call_id}'"))
    })?)
    .clone();
    if matching.next().is_some() {
        return Err(AiMuxError::InvalidArgument(format!(
            "result: tool call id '{tool_call_id}' is not unique"
        )));
    }

    let repaired = apply_tool_call_repair(&original, tools, reply)?;
    if repaired.tool_call_id != tool_call_id
        && calls
            .iter()
            .any(|call| call.tool_call_id == repaired.tool_call_id)
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "result: repaired tool call id '{}' already exists",
            repaired.tool_call_id
        )));
    }

    // The replayed transcript must agree with the content, or the next turn
    // sends the model the unrepaired arguments.
    let replay_input =
        crate::response_messages::response_tool_call_input(&repaired.input, repaired.invalid);
    for step in result.steps_mut() {
        for content in &mut step.content {
            let call = match content {
                GenerateContent::ToolCall(call) => call,
                GenerateContent::ToolApprovalRequest(approval) => &mut approval.tool_call,
                _ => continue,
            };
            if call.tool_call_id == tool_call_id {
                *call = repaired.clone();
            }
        }
        for message in &mut step.response.messages {
            let crate::message::MessageContent::Parts(parts) = &mut message.content else {
                continue;
            };
            for part in parts {
                match part {
                    ContentPart::ToolCall {
                        tool_call_id: id,
                        tool_name,
                        input,
                        ..
                    } if id == tool_call_id => {
                        id.clone_from(&repaired.tool_call_id);
                        tool_name.clone_from(&repaired.tool_name);
                        input.clone_from(&replay_input);
                    }
                    ContentPart::ToolApprovalRequest {
                        tool_call_id: id, ..
                    } if id == tool_call_id => id.clone_from(&repaired.tool_call_id),
                    _ => {}
                }
            }
        }
    }

    serde_json::to_value(&result)
        .map_err(|error| AiMuxError::InvalidArgument(format!("result: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::GenerateTextResult;
    use crate::message::MessageContent;
    use crate::response_messages::ResponseMessageBuilder;
    use crate::result::ToolApprovalRequestOutput;
    use crate::step_result::{StepModel, StepResponse, StepResult};
    use crate::tool::FunctionTool;
    use crate::types::FinishReasonUnified;

    fn weather_tools() -> Vec<Tool> {
        vec![Tool::Function(FunctionTool::new(
            "weather",
            serde_json::json!({
                "type": "object",
                "properties": { "city": { "type": "string" } },
                "required": ["city"],
            }),
        ))]
    }

    fn raw_call(id: &str, input: &str) -> RawToolCall {
        RawToolCall {
            tool_call_id: id.to_string(),
            tool_name: "weather".to_string(),
            input: input.to_string(),
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        }
    }

    /// A result document as `stream_text().consume()` assembles it, holding
    /// one call the model got wrong (`city` is a number).
    async fn result_with_invalid_call(with_approval: bool) -> Value {
        let tools = weather_tools();
        let call = parse_tool_call(
            raw_call("call-1", r#"{"city":7}"#),
            Some(&tools),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(call.invalid, Some(true));
        let mut builder = ResponseMessageBuilder::default();
        builder.tool_call(&call);
        if with_approval {
            builder.approval(&ToolApprovalRequestOutput {
                approval_id: "approval-1".to_string(),
                tool_call: call.clone(),
                reason: None,
                is_automatic: None,
                signature: None,
                input_schema_input: None,
            });
        }
        let assembled = builder.finish();
        let step = StepResult {
            call_id: "call".to_string(),
            step_number: 0,
            model: StepModel {
                provider: "test".to_string(),
                model_id: "model".to_string(),
            },
            content: assembled.content,
            finish_reason: FinishReasonUnified::ToolCalls,
            raw_finish_reason: None,
            usage: crate::types::LanguageModelUsage::default(),
            performance: crate::step_result::StepResultPerformance::default(),
            warnings: Vec::new(),
            request: crate::shared::RequestInfo::default(),
            response: StepResponse {
                messages: assembled.messages,
                ..StepResponse::default()
            },
            provider_metadata: None,
        };
        serde_json::to_value(GenerateTextResult::from_steps(
            vec![step],
            crate::generate::OutputState::NotParsed,
        ))
        .unwrap()
    }

    fn repair(result: &Value, tool_call_id: &str) -> GenerateTextResult {
        let patched = apply_tool_call_repair_to_result(
            result,
            Some(&weather_tools()),
            "call-1",
            ToolCallRepairReply::Repaired {
                tool_call: raw_call(tool_call_id, r#"{"city":"Tokyo"}"#),
            },
        )
        .unwrap();
        serde_json::from_value(patched).unwrap()
    }

    fn content_calls(result: &GenerateTextResult) -> Vec<&ToolCall> {
        result
            .content()
            .into_iter()
            .filter_map(|content| match content {
                GenerateContent::ToolCall(call) => Some(call),
                GenerateContent::ToolApprovalRequest(approval) => Some(&approval.tool_call),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_repair_also_updates_the_content_call() {
        let repaired = repair(&result_with_invalid_call(false).await, "call-1");

        let expected = serde_json::to_value(repaired.tool_calls()[0]).unwrap();
        assert_eq!(expected["input"], serde_json::json!({ "city": "Tokyo" }));
        let calls = content_calls(&repaired);
        assert_eq!(calls.len(), 1);
        assert_eq!(serde_json::to_value(calls[0]).unwrap(), expected);
    }

    #[tokio::test]
    async fn a_renaming_repair_updates_approval_requests() {
        let repaired = repair(&result_with_invalid_call(true).await, "call-2");

        let expected = serde_json::to_value(repaired.tool_calls()[0]).unwrap();
        let calls = content_calls(&repaired);
        assert_eq!(calls.len(), 2);
        for call in calls {
            assert_eq!(serde_json::to_value(call).unwrap(), expected);
        }
        let MessageContent::Parts(parts) = &repaired.response_messages()[0].content else {
            panic!("expected content parts");
        };
        let ids: Vec<_> = parts
            .iter()
            .map(|part| match part {
                ContentPart::ToolCall { tool_call_id, .. }
                | ContentPart::ToolApprovalRequest { tool_call_id, .. } => tool_call_id.as_str(),
                other => panic!("unexpected part {other:?}"),
            })
            .collect();
        assert_eq!(ids, ["call-2", "call-2"]);
    }
}
