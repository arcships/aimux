//! Parse and validate provider tool calls at the Core boundary.
//!
//! Rust port of the AI SDK's `parse-tool-call.ts`
//! (`packages/ai/src/generate-text/parse-tool-call.ts`). Providers deliver the
//! model's raw argument text; Core owns JSON parsing, prototype-pollution
//! rejection, schema validation, and the one-shot repair callback.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::content::{AssistantContentPart, ToolContentPart};
use crate::error::{AiMuxError, ErrorCause, SourceError};
use crate::output::{json_parse_error, type_validation_error, validator_error};
use crate::result::ContentPart;
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

/// A one-shot repair hook for invalid tool calls.
///
/// The Rust form of the AI SDK's `experimental_repairToolCall`. Core calls
/// [`repair`](Self::repair) at most once per invalid call, and parses and
/// validates the returned call from scratch. Returning `Ok(None)` keeps the
/// original validation error; an `Err` becomes a `ToolCallRepair` error that
/// keeps both the original failure and the cause.
///
/// Any `Fn(ToolCallRepairContext) -> impl Future<Output = Result<Option<RawToolCall>, AiMuxError>>`
/// implements this trait, so a closure works wherever a hook does.
///
/// ```
/// use aimux_core::error::AiMuxError;
/// use aimux_core::parse_tool_call::{RepairToolCall, ToolCallRepair, ToolCallRepairContext};
/// use aimux_core::tool::RawToolCall;
///
/// struct RenameTool(&'static str);
///
/// #[async_trait::async_trait]
/// impl RepairToolCall for RenameTool {
///     async fn repair(
///         &self,
///         context: ToolCallRepairContext,
///     ) -> Result<Option<RawToolCall>, AiMuxError> {
///         Ok(Some(RawToolCall {
///             tool_name: self.0.into(),
///             ..context.tool_call
///         }))
///     }
/// }
///
/// let repair = ToolCallRepair::from(RenameTool("weather"));
/// ```
#[async_trait]
pub trait RepairToolCall: Send + Sync {
    /// Return a replacement call, `None` to keep the original error, or an
    /// error to report the repair as failed.
    async fn repair(
        &self,
        context: ToolCallRepairContext,
    ) -> Result<Option<RawToolCall>, AiMuxError>;
}

#[async_trait]
impl<F, Fut> RepairToolCall for F
where
    F: Fn(ToolCallRepairContext) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Option<RawToolCall>, AiMuxError>> + Send,
{
    async fn repair(
        &self,
        context: ToolCallRepairContext,
    ) -> Result<Option<RawToolCall>, AiMuxError> {
        self(context).await
    }
}

/// A shared [`RepairToolCall`] hook, as carried by `GenerateTextOptions`.
///
/// Build one from a closure with [`ToolCallRepair::new`], or from any
/// [`RepairToolCall`] implementation with `From`.
#[derive(Clone)]
pub struct ToolCallRepair(Arc<dyn RepairToolCall>);

impl ToolCallRepair {
    /// Wrap a repair closure.
    #[must_use]
    pub fn new<F, Fut>(repair: F) -> Self
    where
        F: Fn(ToolCallRepairContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<RawToolCall>, AiMuxError>> + Send + 'static,
    {
        Self(Arc::new(repair))
    }

    async fn repair(
        &self,
        context: ToolCallRepairContext,
    ) -> Result<Option<RawToolCall>, AiMuxError> {
        self.0.repair(context).await
    }
}

impl<T: RepairToolCall + 'static> From<T> for ToolCallRepair {
    fn from(repair: T) -> Self {
        Self(Arc::new(repair))
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
///
/// Only a failing repair is wrapped in [`AiMuxError::ToolCallRepair`]
/// (`ai@7.0.127 parse-tool-call.ts:90-96`). A repaired call that fails to
/// parse or validate yields an invalid call built from the original call,
/// carrying the repaired call's own error unwrapped (`:103-110`, `:112-133`).
fn apply_repair_outcome(
    tool_call: RawToolCall,
    original_error: AiMuxError,
    tools: &[Tool],
    outcome: RepairOutcome,
) -> ToolCall {
    let cause = match outcome {
        RepairOutcome::Repaired(repaired) => match parse_and_validate_tool_call(&repaired, tools) {
            Ok((input, dynamic)) => return parsed_tool_call(repaired, input, dynamic),
            Err(repaired_error) => return invalid_tool_call(tool_call, repaired_error),
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
    let invalid = |cause: ErrorCause| AiMuxError::InvalidToolInput {
        tool_name: tool_call.tool_name.clone(),
        tool_input: tool_call.input.clone(),
        cause,
    };
    let validator = jsonschema::validator_for(&function_tool.input_schema).map_err(|error| {
        invalid(
            SourceError {
                message: format!("input schema is invalid: {error}"),
                cause: Some(Box::new(validator_error(&error))),
            }
            .into(),
        )
    })?;
    // Cause wording matches the AI SDK's `TypeValidationError` template;
    // `Value` Display is compact JSON, the `JSON.stringify` equivalent.
    validator.validate(&input).map_err(|error| {
        invalid(
            type_validation_error(&input, &error.to_string(), Some(validator_error(&error))).into(),
        )
    })?;
    Ok((input, None))
}

fn parse_json_input(tool_call: &RawToolCall) -> Result<Value, AiMuxError> {
    if tool_call.input.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    // The cause is the AI SDK's `JSONParseError`.
    let invalid = |parse_error: AiMuxError| AiMuxError::InvalidToolInput {
        tool_name: tool_call.tool_name.clone(),
        tool_input: tool_call.input.clone(),
        cause: parse_error.into(),
    };
    let value: Value = parse_input_text(&tool_call.input).map_err(|error| {
        invalid(json_parse_error(
            &tool_call.input,
            &error.to_string(),
            Some(&error),
        ))
    })?;
    if contains_forbidden_prototype(&value) {
        return Err(invalid(json_parse_error(
            &tool_call.input,
            "Object contains forbidden prototype property",
            None,
        )));
    }
    Ok(value)
}

/// Parse tool-call argument text into the value `JSON.parse` gives, as far
/// as its JSON goes.
///
/// JavaScript has one number type, so `JSON.stringify(JSON.parse("1e2"))` is
/// `100`. serde_json keeps a number written with a fraction or an exponent
/// (`1e2`, `100.0`) as an `f64`, which it writes back as `100.0`. Each such
/// number that is integral is stored as the integer JavaScript prints (see
/// [`js_integer`]); every other number keeps serde_json's form.
fn parse_input_text(text: &str) -> serde_json::Result<Value> {
    let mut value: Value = serde_json::from_str(text)?;
    integral_floats_as_integers(&mut value);
    Ok(value)
}

fn integral_floats_as_integers(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(integer) = js_integer(number) {
                *number = integer;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(integral_floats_as_integers),
        Value::Object(map) => map.values_mut().for_each(integral_floats_as_integers),
        _ => {}
    }
}

/// The integer `JSON.stringify` writes for an integral, finite `f64`.
///
/// Below 1e21 JavaScript writes such a number without an exponent: its
/// shortest round-trip digits, then zeros (ECMA-262 `Number::toString`, step
/// 7). Rust's `Display` for `f64` writes the same digits, so `2 ** 60` is
/// `1152921504606847000` in both. `-0` is written `0`. `None` when the
/// number is not such a float, or when its digits do not fit an `i64` or a
/// `u64` (from about 1.8e19 up to 1e21), since serde_json has no larger
/// integer.
fn js_integer(number: &serde_json::Number) -> Option<serde_json::Number> {
    if !number.is_f64() {
        return None;
    }
    let float = number.as_f64()?;
    if !float.is_finite() || float.fract() != 0.0 || float.abs() >= 1e21 {
        return None;
    }
    if float == 0.0 {
        return Some(0.into());
    }
    let digits = float.to_string();
    digits
        .parse::<i64>()
        .map(Into::into)
        .or_else(|_| digits.parse::<u64>().map(Into::into))
        .ok()
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
    // AI SDK's `parseToolCall` catch-all does (`parse-tool-call.ts:115-117`,
    // `safeParseJSON`): the parsed value when the text is valid JSON without
    // forbidden prototype keys, the verbatim text otherwise. Callers
    // replaying an invalid call rely on this — `response_messages` only
    // carries a structured input into the next turn's transcript, so an
    // unknown tool called with perfectly good arguments must still parse
    // here.
    let input = back_filled_input(&tool_call.input);
    ToolCall {
        invalid: Some(true),
        error: Some(error),
        ..parsed_tool_call(tool_call, input, Some(true))
    }
}

/// The `input` an invalid call carries for its argument text.
fn back_filled_input(text: &str) -> Value {
    parse_input_text(text)
        .ok()
        .filter(|value| !contains_forbidden_prototype(value))
        .unwrap_or_else(|| Value::String(text.to_owned()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Host-side tool-call repair (RFC-0035)
// ─────────────────────────────────────────────────────────────────────────────

/// A host's answer to one tool-call repair request.
///
/// The bindings cannot carry a [`ToolCallRepair`] closure, so a host instead
/// reads the invalid call off the result, repairs it in its own language, and
/// hands the answer back as this value. Wire shape:
/// `{"type":"repaired","toolCall":{…}}`, `{"type":"unchanged"}`, or
/// `{"type":"failed","message":"…"}`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[cfg_attr(feature = "ts-export", ts(export))]
pub enum ToolCallRepairReply {
    /// Re-validate this replacement call, exactly as the closure path does
    /// for a returned `Some(call)`.
    ///
    /// The call may carry its `type` tag, as the AI SDK repair function's
    /// `LanguageModelV4ToolCall` does.
    Repaired {
        #[serde(deserialize_with = "crate::wire::typed::deserialize")]
        tool_call: RawToolCall,
    },
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
                RepairOutcome::Failed(AiMuxError::other(message))
            }
        }
    }
}

/// The provider's verbatim argument text of an invalid call, when its error
/// kept it.
///
/// The error of a call whose repair was rejected by validation holds the
/// replacement's text, not the original's (it is reported unwrapped, as
/// upstream), so the error's text counts only when it back-fills to the
/// call's own `input`.
pub(crate) fn original_tool_call_text(input: &Value, error: &AiMuxError) -> Option<String> {
    fn error_text(error: &AiMuxError) -> Option<&str> {
        match error {
            AiMuxError::InvalidToolInput { tool_input, .. } => Some(tool_input),
            AiMuxError::NoSuchTool { tool_input, .. } => tool_input.as_deref(),
            AiMuxError::ToolCallRepair { original_error, .. } => error_text(original_error),
            _ => None,
        }
    }
    error_text(error)
        .filter(|text| back_filled_input(text) == *input)
        .map(str::to_owned)
}

/// Reconstruct the provider-facing call an invalid [`ToolCall`] was built
/// from.
///
/// The argument text comes from the error, which keeps it verbatim.
/// [`invalid_tool_call`] parses the text into `input` when it is valid JSON,
/// and that loses the JSON string quoting of a bare literal (`"Tokyo"` would
/// come back as `Tokyo`), duplicate keys, number spelling and whitespace, all
/// of which matter when a host re-submits the text unchanged under a new tool
/// name. Errors without the original text (from before the text was
/// recorded, or the error of a rejected repair) fall back to `input`.
///
/// `dynamic` is not recovered: an invalid call always reports `Some(true)`.
/// That only matters for re-deriving the original error, which is read off the
/// call instead.
fn raw_from_invalid(tool_call: &ToolCall) -> Result<(RawToolCall, AiMuxError), AiMuxError> {
    if tool_call.invalid != Some(true) {
        return Err(AiMuxError::invalid_argument(format!(
            "tool call '{}' is not invalid; nothing to repair",
            tool_call.tool_call_id
        )));
    }
    let error = tool_call.error.clone().ok_or_else(|| {
        AiMuxError::invalid_argument(format!(
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
            input: original_tool_call_text(&tool_call.input, &error).unwrap_or_else(|| {
                match &tool_call.input {
                    Value::String(text) => text.clone(),
                    input => input.to_string(),
                }
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
        AiMuxError::invalid_argument(
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
/// Mirrors the AI SDK `repairToolCall` argument: `{toolCall, error,
/// inputSchema, tools, messages, instructions}`. `toolCall` is the
/// provider-facing call (raw argument *text*), `inputSchema` the schema of
/// the named tool (an empty-object schema when the name does not resolve),
/// and `error` the serialized failure the call already carries.
///
/// `tools` is `Option` for the same reason [`parse_tool_call`]'s is: the AI
/// SDK never repairs a call made without a tool set. That is not a failure —
/// it is the answer "this call is not repairable", and the caller skips it.
///
/// # Errors
///
/// [`AiMuxError::invalid_argument`] when `tool_call` is not an invalid call, or
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
        "toolCall": crate::wire::Tagged(&context.tool_call),
        "error": context.error,
        "inputSchema": context.input_schema(&context.tool_call.tool_name),
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
/// [`AiMuxError::invalid_argument`] when `tools` is `None` (the call was never
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
/// renames the response messages' approval request. The tool error text
/// generation adds for a call that could not be parsed follows the repaired
/// call: rewritten while the call is still invalid, removed (with its result
/// in the tool message) once it is valid. The derived values of
/// the returned document (`toolCalls`, `content`, `responseMessages`, ...)
/// are written from the patched steps.
///
/// The OpenAI-shaped `ChatCompletion` is deliberately not supported: it drops
/// `invalid` and `error`, so a host cannot tell from it that a call needs
/// repairing in the first place. Repair the native result and convert.
///
/// # Errors
///
/// [`AiMuxError::invalid_argument`] when `tools` is `None`, when the document
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
        .map_err(|error| AiMuxError::InvalidArgument {
            message: format!("result: {error}"),
            cause: Some(ErrorCause::source(&error)),
        })?;

    // Calls and the transcript are keyed by id here, so a duplicated id
    // cannot be patched unambiguously; refuse rather than rewrite the first
    // entry twice.
    let calls = result.tool_calls();
    let mut matching = calls
        .iter()
        .filter(|call| call.tool_call_id == tool_call_id);
    let original = (*matching.next().ok_or_else(|| {
        AiMuxError::invalid_argument(format!("result: no tool call '{tool_call_id}'"))
    })?)
    .clone();
    if matching.next().is_some() {
        return Err(AiMuxError::invalid_argument(format!(
            "result: tool call id '{tool_call_id}' is not unique"
        )));
    }

    let repaired = apply_tool_call_repair(&original, tools, reply)?;
    if repaired.tool_call_id != tool_call_id
        && calls
            .iter()
            .any(|call| call.tool_call_id == repaired.tool_call_id)
    {
        return Err(AiMuxError::invalid_argument(format!(
            "result: repaired tool call id '{}' already exists",
            repaired.tool_call_id
        )));
    }

    // The replayed transcript must agree with the content, or the next turn
    // sends the model the unrepaired arguments.
    let replay_input =
        crate::response_messages::response_tool_call_input(&repaired.input, repaired.invalid);
    // Text generation answers a call that could not be parsed with a tool
    // error (`generate-text.ts:1303-1323`). After the repair the error
    // follows the repaired call: it is rewritten when the call is still
    // invalid and dropped when it is valid.
    let tool_error = crate::generate::invalid_tool_call_error(&repaired)
        .filter(|_| repaired.dynamic == Some(true));
    for step in result.steps_mut() {
        let mut had_tool_error = false;
        step.content.retain_mut(|content| {
            let call = match content {
                ContentPart::ToolCall(call) => call,
                ContentPart::ToolApprovalRequest(approval) => &mut approval.tool_call,
                ContentPart::ToolError(error)
                    if error.tool_call_id == tool_call_id
                        && error.provider_executed != Some(true) =>
                {
                    had_tool_error = true;
                    return match &tool_error {
                        Some(tool_error) => {
                            error.clone_from(tool_error);
                            true
                        }
                        None => false,
                    };
                }
                _ => return true,
            };
            if call.tool_call_id == tool_call_id {
                *call = repaired.clone();
            }
            true
        });
        for message in &mut step.response.messages {
            match message {
                crate::message::ModelMessage::Assistant {
                    content: crate::message::AssistantContent::Parts(parts),
                    ..
                } => {
                    for part in parts {
                        match part {
                            AssistantContentPart::ToolCall {
                                tool_call_id: id,
                                tool_name,
                                input,
                                ..
                            } if id == tool_call_id => {
                                id.clone_from(&repaired.tool_call_id);
                                tool_name.clone_from(&repaired.tool_name);
                                input.clone_from(&replay_input);
                            }
                            AssistantContentPart::ToolApprovalRequest {
                                tool_call_id: id, ..
                            } if id == tool_call_id => id.clone_from(&repaired.tool_call_id),
                            _ => {}
                        }
                    }
                }
                crate::message::ModelMessage::Tool { content, .. } if had_tool_error => {
                    content.retain_mut(|part| match part {
                        ToolContentPart::ToolResult(result)
                            if result.tool_call_id == tool_call_id =>
                        {
                            match &tool_error {
                                Some(tool_error) => {
                                    result.tool_call_id.clone_from(&tool_error.tool_call_id);
                                    result.tool_name = Some(tool_error.tool_name.clone());
                                    result.output = crate::content::ToolResultOutput::from_result(
                                        tool_error.error.clone(),
                                        true,
                                    );
                                    true
                                }
                                None => false,
                            }
                        }
                        _ => true,
                    });
                }
                _ => {}
            }
        }
        step.response.messages.retain(|message| {
            !matches!(message, crate::message::ModelMessage::Tool { content, .. } if content.is_empty())
        });
    }

    serde_json::to_value(&result).map_err(|error| AiMuxError::InvalidArgument {
        message: format!("result: {error}"),
        cause: Some(ErrorCause::source(&error)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::GenerateTextResult;
    use crate::message::{AssistantContent, ModelMessage};
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
        result_with_invalid_call_and_error(with_approval, false).await
    }

    /// The same document, optionally with the tool error `generate_text`
    /// adds after the content for the invalid call.
    async fn result_with_invalid_call_and_error(with_approval: bool, with_error: bool) -> Value {
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
        if with_error {
            builder.tool_error(crate::generate::invalid_tool_call_error(&call).unwrap());
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
                id: "response".to_string(),
                timestamp: crate::timestamp::Timestamp::now(),
                model_id: "model".to_string(),
                headers: None,
                body: None,
                messages: assembled.messages,
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
                ContentPart::ToolCall(call) => Some(call),
                ContentPart::ToolApprovalRequest(approval) => Some(&approval.tool_call),
                _ => None,
            })
            .collect()
    }

    /// The AI SDK parses tool input with `JSON.parse` (`ai`
    /// `generate-text/parse-tool-call.ts:116`, `:256`; `provider-utils`
    /// `parse-json.ts:97`, `secure-json-parse.ts:31`), so an integral number
    /// written with a fraction or exponent is a plain integer in its JSON:
    /// `JSON.stringify(JSON.parse('{"a":1e2}'))` is `{"a":100}`.
    #[tokio::test]
    async fn integral_input_numbers_are_written_as_javascript_writes_them() {
        let tools = weather_tools();
        let text =
            r#"{"city":"SG","n":[1e2,100.0,-2.0E1,1.5,0.0,-0.0,9007199254740993.0,1e300,7]}"#;
        let call = parse_tool_call(raw_call("call-1", text), Some(&tools), None, &[], None).await;
        assert_eq!(call.invalid, None);
        // `JSON.stringify(JSON.parse(text))` in Node.
        assert_eq!(
            serde_json::to_string(&call.input).unwrap(),
            r#"{"city":"SG","n":[100,100,-20,1.5,0,0,9007199254740992,1e+300,7]}"#
        );

        // An invalid call keeps the parsed value, normalized the same way.
        let invalid = parse_tool_call(
            raw_call("call-2", r#"{"town":1e1}"#),
            Some(&tools),
            None,
            &[],
            None,
        )
        .await;
        assert_eq!(invalid.invalid, Some(true));
        assert_eq!(
            serde_json::to_string(&invalid.input).unwrap(),
            r#"{"town":10}"#
        );

        // Shortest round-trip digits then zeros, as `Number::toString` (2 ** 60).
        let number = |text: &str| js_integer(&serde_json::from_str(text).unwrap());
        assert_eq!(
            number("1152921504606846976.0"),
            Some(1_152_921_504_606_847_000_u64.into())
        );
        assert_eq!(
            number("-1e18"),
            Some((-1_000_000_000_000_000_000_i64).into())
        );
        // Past `u64` serde_json has no integer; the float is kept.
        assert_eq!(number("1e20"), None);
        assert_eq!(number("12"), None);
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
        let ModelMessage::Assistant {
            content: AssistantContent::Parts(parts),
            ..
        } = &repaired.response_messages()[0]
        else {
            panic!("expected content parts");
        };
        let ids: Vec<_> = parts
            .iter()
            .map(|part| match part {
                AssistantContentPart::ToolCall { tool_call_id, .. }
                | AssistantContentPart::ToolApprovalRequest { tool_call_id, .. } => {
                    tool_call_id.as_str()
                }
                other => panic!("unexpected part {other:?}"),
            })
            .collect();
        assert_eq!(ids, ["call-2", "call-2"]);
    }

    /// A repaired call no longer carries the tool error text generation
    /// added for it (`generate-text.ts:1303-1323`): the content's tool error
    /// and the tool message holding its error result are removed.
    #[tokio::test]
    async fn a_repair_removes_the_tool_error_of_the_invalid_call() {
        let document = result_with_invalid_call_and_error(false, true).await;
        let before: GenerateTextResult = serde_json::from_value(document.clone()).unwrap();
        assert!(matches!(
            before.content().last(),
            Some(ContentPart::ToolError(_))
        ));
        assert!(matches!(
            before.response_messages().last(),
            Some(ModelMessage::Tool { .. })
        ));

        let repaired = repair(&document, "call-1");
        assert!(
            !repaired
                .content()
                .iter()
                .any(|part| matches!(part, ContentPart::ToolError(_)))
        );
        assert_eq!(repaired.response_messages().len(), 1);
        assert!(matches!(
            repaired.response_messages()[0],
            ModelMessage::Assistant { .. }
        ));
    }

    /// A repair the tool still rejects leaves the call invalid; its tool
    /// error is then the one the repaired call gets.
    #[tokio::test]
    async fn a_rejected_repair_rewrites_the_tool_error() {
        let document = result_with_invalid_call_and_error(false, true).await;
        let patched = apply_tool_call_repair_to_result(
            &document,
            Some(&weather_tools()),
            "call-1",
            ToolCallRepairReply::Repaired {
                tool_call: raw_call("call-1", r#"{"city":8}"#),
            },
        )
        .unwrap();
        let repaired: GenerateTextResult = serde_json::from_value(patched).unwrap();
        let call = repaired.tool_calls()[0];
        assert_eq!(call.invalid, Some(true));
        let errors: Vec<_> = repaired
            .content()
            .into_iter()
            .filter_map(|part| match part {
                ContentPart::ToolError(error) => Some(error),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            Some(errors[0]),
            crate::generate::invalid_tool_call_error(call).as_ref()
        );
        let Some(ModelMessage::Tool { content, .. }) = repaired.response_messages().last() else {
            panic!("expected a tool message");
        };
        assert_eq!(content.len(), 1);
    }
}
