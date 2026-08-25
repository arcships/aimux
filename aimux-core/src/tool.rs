//! Tool / function-calling types.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::types::ProviderMetadata;

/// A tool definition passed to the model in `CallOptions.tools`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FunctionTool {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the tool's parameters.
    pub input_schema: Value,
    /// Whether the tool supports strict schema enforcement (OpenAI strict mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    /// Provider-specific options, keyed by provider name
    /// (e.g. `{"anthropic": {"eagerInputStreaming": true}}`). Aligned with the
    /// V4 `providerOptions` field on function tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<HashMap<String, Value>>,
    /// Example inputs for the tool (V4 `inputExamples`), used by some providers
    /// to emit `input_examples` in the request body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_examples: Option<Vec<Value>>,
}

impl FunctionTool {
    /// Create a `FunctionTool` with the required fields and all optional fields
    /// set to `None`. Combined with struct-update syntax this avoids spelling
    /// out every `None` field in tests.
    pub fn new(name: impl Into<String>, input_schema: Value) -> Self {
        FunctionTool {
            name: name.into(),
            description: None,
            input_schema,
            strict: None,
            provider_options: None,
            input_examples: None,
        }
    }

    /// Builder-style setter for `description`.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Builder-style setter for `strict`.
    #[must_use]
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = Some(strict);
        self
    }
}

/// A provider-defined tool (e.g. `anthropic.web_search_20250305`,
/// `google.googleSearch`). Mirrors the V4 `LanguageModelV4ProviderTool`.
///
/// Provider tools are specific to a certain provider. The input and output
/// schemas are defined by the provider, and some are executed on the provider's
/// servers (e.g. web search, code execution).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderTool {
    /// The ID of the tool, following the format `<provider-id>.<unique-tool-name>`.
    pub id: String,
    /// The name of the tool, unique within this model call.
    pub name: String,
    /// The arguments for configuring the tool. Must match the expected
    /// arguments defined by the provider for this tool.
    pub args: Value,
}

/// A tool that can be either a user-defined function tool or a
/// provider-defined tool. Mirrors the V4
/// `Array<LanguageModelV4FunctionTool | LanguageModelV4ProviderTool>`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Tool {
    /// A user-defined function tool.
    Function(FunctionTool),
    /// A provider-defined tool.
    Provider(ProviderTool),
}

impl From<FunctionTool> for Tool {
    fn from(ft: FunctionTool) -> Self {
        Tool::Function(ft)
    }
}

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

fn invalid_tool_call(tool_call: RawToolCall, error: AiMuxError) -> ToolCall {
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

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolCall {
    /// Provider-assigned call id.
    pub tool_call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Parsed arguments, or the original string when `invalid` is true and the
    /// provider input was not valid JSON.
    pub input: Value,
    /// Whether the tool call will be executed by the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_executed: Option<bool>,
    /// Whether the tool is dynamic (defined at runtime, e.g. MCP tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<bool>,
    /// Provider-assigned thought signature (e.g. Google Gemini
    /// `thoughtSignature`). Must be echoed back verbatim on the follow-up turn
    /// when the tool result is sent; thinking models reject the request
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    /// Additional provider-specific metadata associated with this call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ProviderMetadata>,
    /// Set when lookup, JSON parsing, or schema validation still failed after
    /// the optional repair attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid: Option<bool>,
    /// Typed failure associated with an invalid tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AiMuxError>,
}

/// The result of executing a tool call, to be sent back to the model.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolResult {
    /// Must match the corresponding `ToolCall::tool_call_id`.
    pub tool_call_id: String,
    /// The tool's output (usually a JSON-serializable value or plain text).
    pub result: Value,
    /// Whether the result is an error or error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Whether the result is preliminary (replaces prior, e.g. image previews).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preliminary: Option<bool>,
}

/// How the model should choose tools.
///
/// Wire format aligns with Vercel AI SDK `toolChoice`:
/// `"auto" | "none" | "required" | { "type": "tool", "toolName": "..." }`.
/// This is a mixed tagged/untagged shape (unit variants as bare strings,
/// `Tool` variant as a tagged object), so serde is implemented by hand.
#[derive(Debug, Clone, Default, PartialEq, TS)]
#[ts(
    export,
    type = "\"auto\" | \"none\" | \"required\" | { type: \"tool\"; toolName: string }"
)]
pub enum ToolChoice {
    /// Model decides whether to call a tool (default).
    #[default]
    Auto,
    /// Model must not call any tools.
    None,
    /// Model must call at least one tool.
    Required,
    /// Model must call the specified tool.
    Tool { tool_name: String },
}

impl Serialize for ToolChoice {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        match self {
            ToolChoice::Auto => serializer.serialize_str("auto"),
            ToolChoice::None => serializer.serialize_str("none"),
            ToolChoice::Required => serializer.serialize_str("required"),
            ToolChoice::Tool { tool_name } => {
                let mut s = serializer.serialize_struct("ToolChoice", 2)?;
                s.serialize_field("type", "tool")?;
                s.serialize_field("toolName", tool_name)?;
                s.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ToolChoice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            Tool {
                #[serde(rename = "type")]
                typ: String,
                #[serde(rename = "toolName")]
                tool_name: Option<String>,
            },
        }

        match Repr::deserialize(deserializer)? {
            Repr::Str(s) => match s.as_str() {
                "auto" => Ok(ToolChoice::Auto),
                "none" => Ok(ToolChoice::None),
                "required" => Ok(ToolChoice::Required),
                other => Err(D::Error::custom(format!(
                    "unknown toolChoice '{other}' (expected auto/none/required or {{type:\"tool\",toolName}})"
                ))),
            },
            Repr::Tool { typ, tool_name } => {
                if typ != "tool" {
                    return Err(D::Error::custom(format!(
                        "unknown toolChoice type '{typ}' (expected \"tool\")"
                    )));
                }
                let tool_name = tool_name.ok_or_else(|| {
                    D::Error::custom("toolChoice {type:\"tool\"} requires a toolName field")
                })?;
                Ok(ToolChoice::Tool { tool_name })
            }
        }
    }
}
