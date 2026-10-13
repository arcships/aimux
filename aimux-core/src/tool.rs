//! Tool / function-calling types.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::error::AiMuxError;
use crate::shared::SharedProviderOptions;
use crate::types::{NamespacedKind, ProviderMetadata};

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
    pub provider_options: Option<SharedProviderOptions>,
    /// Example inputs for the tool (V4 `inputExamples`), used by some providers
    /// to emit `input_examples` in the request body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_examples: Option<Vec<FunctionToolInputExample>>,
}

/// An example input for a function tool.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FunctionToolInputExample {
    pub input: Map<String, Value>,
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
/// `google.googleSearch`). Mirrors the V4 `LanguageModelV4ProviderTool`
/// (`@ai-sdk/provider@4.0.21
/// src/language-model/v4/language-model-v4-provider-tool.ts:8-28`).
///
/// Provider tools are specific to a certain provider. The input and output
/// schemas are defined by the provider, and some are executed on the provider's
/// servers (e.g. web search, code execution).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProviderTool {
    /// The ID of the tool, following the format `<provider-id>.<unique-tool-name>`.
    pub id: NamespacedKind,
    /// The name of the tool, unique within this model call.
    pub name: String,
    /// The arguments for configuring the tool. Must match the expected
    /// arguments defined by the provider for this tool.
    pub args: Map<String, Value>,
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
///
/// The wire shape matches [`ToolCall`] field for field, except that `input`
/// is the provider's raw argument *text* rather than a parsed value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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
    pub provider_metadata: Option<ProviderMetadata>,
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

/// A tool result produced by a provider-executed tool (e.g. xAI `file_search`,
/// web search), carried next to the [`RawToolCall`] it answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ToolResult {
    /// Must match the corresponding call's `tool_call_id`.
    pub tool_call_id: String,
    pub tool_name: String,
    /// The tool's output: any JSON value but `null`, as the V4
    /// `result: NonNullable<JSONValue>` (`@ai-sdk/provider@4.0.21
    /// src/language-model/v4/language-model-v4-tool-result.ts:23`); a `null`
    /// result does not deserialize.
    #[serde(deserialize_with = "non_null_json")]
    pub result: Value,
    /// Whether the result is an error or error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Whether the result is preliminary (replaces prior, e.g. image previews).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preliminary: Option<bool>,
    /// Whether the tool is dynamic (defined at runtime, e.g. MCP tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<bool>,
    /// Additional provider-specific metadata for the tool result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<ProviderMetadata>,
}

/// A JSON value other than `null` (`NonNullable<JSONValue>`).
fn non_null_json<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Null => Err(serde::de::Error::custom(
            "a tool result's `result` must not be null",
        )),
        value => Ok(value),
    }
}

/// How the model should choose tools, as the caller gives it
/// (`GenerateTextOptions.tool_choice`).
///
/// The AI SDK's user-level `ToolChoice`
/// (`ai@7.0.127 src/types/language-model.ts:102-106`):
/// `"auto" | "none" | "required" | { "type": "tool", "toolName": "..." }`.
/// This is a mixed tagged/untagged shape (unit variants as bare strings,
/// `Tool` variant as a tagged object), so serde is implemented by hand.
/// [`prepare_tool_choice`] turns it into the [`LanguageModelV4ToolChoice`]
/// a model call carries.
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

/// How a model call should choose tools: the V4 `LanguageModelV4ToolChoice`
/// (`@ai-sdk/provider@4.0.21
/// src/language-model/v4/language-model-v4-tool-choice.ts:1-5`), carried by
/// [`CallOptions::tool_choice`](crate::options::CallOptions::tool_choice).
///
/// Always an object tagged by `type`, unlike the user-level [`ToolChoice`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[ts(export)]
pub enum LanguageModelV4ToolChoice {
    /// The model decides whether to call a tool.
    Auto,
    /// The model must not call a tool.
    None,
    /// The model must call one of the available tools.
    Required,
    /// The model must call the named tool.
    Tool {
        #[serde(rename = "toolName")]
        tool_name: String,
    },
}

/// The AI SDK's `prepareToolChoice`
/// (`ai@7.0.127 src/prompt/prepare-tool-choice.ts:4-15`): no choice is
/// `{type:"auto"}`, a string becomes `{type:<string>}`, and a tool choice
/// keeps its `toolName`.
#[must_use]
pub fn prepare_tool_choice(tool_choice: Option<ToolChoice>) -> LanguageModelV4ToolChoice {
    match tool_choice {
        None | Some(ToolChoice::Auto) => LanguageModelV4ToolChoice::Auto,
        Some(ToolChoice::None) => LanguageModelV4ToolChoice::None,
        Some(ToolChoice::Required) => LanguageModelV4ToolChoice::Required,
        Some(ToolChoice::Tool { tool_name }) => LanguageModelV4ToolChoice::Tool { tool_name },
    }
}
