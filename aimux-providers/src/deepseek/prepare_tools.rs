//! DeepSeek tool preparation (`deepseek-prepare-tools.ts`).

use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
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
///
/// # Errors
///
/// `UnsupportedFunctionality` when a tool is `strict` and the base URL is not
/// the beta one, or when strict and non-strict function tools are mixed.
pub(crate) fn prepare_tools(
    tools: Option<&Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
    supports_strict_tool_calls: bool,
) -> Result<PreparedTools, AiMuxError> {
    let mut tool_warnings = Vec::new();
    let Some(tools) = tools.filter(|tools| !tools.is_empty()) else {
        return Ok(PreparedTools {
            tools: None,
            tool_choice: None,
            tool_warnings,
        });
    };

    let function_tools = || {
        tools.iter().filter_map(|tool| match tool {
            Tool::Function(tool) => Some(tool),
            Tool::Provider(_) => None,
        })
    };
    let has_strict_tool = function_tools().any(|tool| tool.strict == Some(true));
    if has_strict_tool && !supports_strict_tool_calls {
        return Err(AiMuxError::UnsupportedFunctionality(
            "DeepSeek strict tool calls require a beta base URL ending in `/beta`.".to_string(),
        ));
    }
    if has_strict_tool
        && supports_strict_tool_calls
        && function_tools().any(|tool| tool.strict != Some(true))
    {
        return Err(AiMuxError::UnsupportedFunctionality(
            "DeepSeek strict mode requires every function tool in the request to set `strict: true`."
                .to_string(),
        ));
    }

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

    let tool_choice = tool_choice.map(|choice| match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool { tool_name } => {
            json!({ "type": "function", "function": { "name": tool_name } })
        }
    });
    Ok(PreparedTools {
        tools: Some(deepseek_tools),
        tool_choice,
        tool_warnings,
    })
}
