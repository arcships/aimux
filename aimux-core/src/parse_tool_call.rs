//! Parse and validate provider tool calls at the Core boundary.
//!
//! Rust port of the AI SDK's `parse-tool-call.ts`
//! (`packages/ai/src/generate-text/parse-tool-call.ts`). Providers deliver the
//! model's raw argument text; Core owns JSON parsing, prototype-pollution
//! rejection, schema validation, and the one-shot repair callback.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::error::AiMuxError;
use crate::tool::{Tool, ToolCall};
use crate::types::ProviderMetadata;

/// Provider-facing tool call before Core parses and validates its input.
#[derive(Debug, Clone)]
pub struct RawToolCall {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: String,
    pub provider_executed: Option<bool>,
    pub dynamic: Option<bool>,
    pub thought_signature: Option<String>,
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
            if let Some(repair_tool_call) = repair_tool_call {
                let context = ToolCallRepairContext {
                    instructions: instructions.map(str::to_owned),
                    system: instructions.map(str::to_owned),
                    messages: messages.to_vec(),
                    tool_call: tool_call.clone(),
                    tools: tools.to_vec(),
                    error: original_error.clone(),
                };
                match repair_tool_call.repair(context).await {
                    Ok(Some(repaired)) => match parse_and_validate_tool_call(&repaired, tools) {
                        Ok((input, dynamic)) => return valid_tool_call(repaired, input, dynamic),
                        Err(repaired_error) => {
                            return invalid_tool_call(tool_call, repaired_error);
                        }
                    },
                    Ok(None) => {}
                    Err(repair_error) => {
                        return invalid_tool_call(
                            tool_call,
                            AiMuxError::ToolCallRepair {
                                original_error: Box::new(original_error),
                                cause: Box::new(repair_error),
                            },
                        );
                    }
                }
            }
            invalid_tool_call(tool_call, original_error)
        }
    }
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

/// Whether `error` means the raw text was never even attempted to parse.
///
/// Currently only `NoSuchTool`: like the AI SDK, aimux raises it purely from
/// the tool name, before looking at the arguments at all — so unlike
/// `InvalidToolInput` (a parse or schema failure), there is no "best effort"
/// parsed value to fall back to. Recurses through `ToolCallRepair` so a
/// `NoSuchTool` that survives a failed repair attempt is still recognized.
fn input_was_never_parsed(error: &AiMuxError) -> bool {
    match error {
        AiMuxError::NoSuchTool { .. } => true,
        AiMuxError::ToolCallRepair { original_error, .. } => input_was_never_parsed(original_error),
        _ => false,
    }
}

fn invalid_tool_call(tool_call: RawToolCall, error: AiMuxError) -> ToolCall {
    let input = if input_was_never_parsed(&error) {
        // Keep the raw text verbatim and unparsed — a valid-JSON-but-quoted
        // call (`"hello"`) must not lose its quotes to a "helpful" parse that
        // was never actually attempted for this error.
        Value::String(tool_call.input.clone())
    } else {
        serde_json::from_str(&tool_call.input)
            .unwrap_or_else(|_| Value::String(tool_call.input.clone()))
    };
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
