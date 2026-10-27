//! `tools` / `tool_choice` of the chat body (`openai-compatible-prepare-tools.ts`).

use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice};
use aimux_core::tool::Tool;
use aimux_core::types::Warning;
use serde_json::{Map, Value, json};

/// `tools` / `tool_choice` ready for the body, with the warnings raised while
/// preparing them.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub warnings: Vec<Warning>,
}

/// The wire form of a function tool.
pub(crate) fn function_tool_value(tool: &aimux_core::tool::FunctionTool) -> Value {
    let mut function = Map::new();
    function.insert("name".into(), json!(tool.name));
    if let Some(description) = &tool.description {
        function.insert("description".into(), json!(description));
    }
    function.insert("parameters".into(), tool.input_schema.clone());
    if let Some(strict) = tool.strict {
        function.insert("strict".into(), json!(strict));
    }
    json!({ "type": "function", "function": function })
}

/// The wire form of the tool choice.
pub(crate) fn tool_choice_value(choice: &LanguageModelV4ToolChoice) -> Value {
    match choice {
        LanguageModelV4ToolChoice::Auto => json!("auto"),
        LanguageModelV4ToolChoice::None => json!("none"),
        LanguageModelV4ToolChoice::Required => json!("required"),
        LanguageModelV4ToolChoice::Tool { tool_name } => {
            json!({ "type": "function", "function": { "name": tool_name } })
        }
    }
}

/// Function tools only; a provider-defined tool is unsupported by the generic
/// endpoint and warned about (the AI SDK's `prepareTools`).
pub(crate) fn prepare_function_tools(options: &CallOptions) -> PreparedTools {
    let mut prepared = PreparedTools::default();
    let Some(tools) = options.tools.as_ref().filter(|tools| !tools.is_empty()) else {
        return prepared;
    };
    let mut wire = Vec::new();
    for tool in tools {
        match tool {
            Tool::Function(function) => wire.push(function_tool_value(function)),
            Tool::Provider(provider) => prepared.warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", provider.id),
                details: None,
            }),
        }
    }
    prepared.tools = Some(wire);
    prepared.tool_choice = options.tool_choice.as_ref().map(tool_choice_value);
    prepared
}
