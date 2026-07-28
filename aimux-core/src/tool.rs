//! Tool / function-calling types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool definition passed to the model in `CallOptions.tools`.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Builder-style setter for `strict`.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned call id.
    pub tool_call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Arguments as a JSON value (usually an object).
    pub input: Value,
}

/// The result of executing a tool call, to be sent back to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Must match the corresponding `ToolCall::tool_call_id`.
    pub tool_call_id: String,
    /// The tool's output (usually a JSON-serializable value or plain text).
    pub output: Value,
}

/// How the model should choose tools.
#[derive(Debug, Clone, Default)]
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
