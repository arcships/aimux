//! DeepSeek tool preparation (`deepseek-prepare-tools.ts`).

use serde_json::{Map, Value, json};

use aimux_core::options::ToolChoice;
use aimux_core::tool::Tool;
use aimux_core::types::Warning;

/// `tools` / `tool_choice` ready for the body, with the warnings raised while
/// preparing them.
pub(crate) struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

/// Function tools go to the body; a provider-defined tool is unsupported and
/// warned about. With no tools there is no tool choice.
pub(crate) fn prepare_tools(tools: Option<&Vec<Tool>>, tool_choice: &ToolChoice) -> PreparedTools {
    let mut tool_warnings = Vec::new();
    let Some(tools) = tools.filter(|tools| !tools.is_empty()) else {
        return PreparedTools {
            tools: None,
            tool_choice: None,
            tool_warnings,
        };
    };

    let mut deepseek_tools = Vec::new();
    for tool in tools {
        match tool {
            Tool::Provider(tool) => tool_warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", tool.id),
                details: None,
            }),
            Tool::Function(tool) => {
                let mut function = Map::new();
                function.insert("name".into(), json!(tool.name));
                if let Some(description) = &tool.description {
                    function.insert("description".into(), json!(description));
                }
                function.insert("parameters".into(), tool.input_schema.clone());
                if let Some(strict) = tool.strict {
                    function.insert("strict".into(), json!(strict));
                }
                deepseek_tools.push(json!({ "type": "function", "function": function }));
            }
        }
    }

    let tool_choice = match tool_choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool { tool_name } => {
            json!({ "type": "function", "function": { "name": tool_name } })
        }
    };
    PreparedTools {
        tools: Some(deepseek_tools),
        tool_choice: Some(tool_choice),
        tool_warnings,
    }
}
