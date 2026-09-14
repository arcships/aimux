//! Tool / function-calling types.

use std::collections::HashMap;

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
