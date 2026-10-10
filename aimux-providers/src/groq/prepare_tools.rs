//! Mirrors `groq-prepare-tools.ts`.

use serde_json::{Map, Value, json};

use aimux_core::options::CallOptions;
use aimux_core::tool::{Tool, ToolChoice};
use aimux_core::types::Warning;

use super::browser_search_models::{
    get_supported_models_string, is_browser_search_supported_model,
};

/// `tools` / `tool_choice` ready for the request body, with the warnings
/// raised while preparing them.
#[derive(Debug, Default)]
pub(crate) struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

pub(crate) fn prepare_tools(options: &CallOptions, model_id: &str) -> PreparedTools {
    let mut prepared = PreparedTools::default();

    // An empty tools array is the same as none, to prevent errors.
    let Some(tools) = options.tools.as_ref().filter(|tools| !tools.is_empty()) else {
        return prepared;
    };

    let mut groq_tools = Vec::new();
    for tool in tools {
        match tool {
            Tool::Provider(tool) if tool.id == "groq.browser_search" => {
                if is_browser_search_supported_model(model_id) {
                    groq_tools.push(json!({ "type": "browser_search" }));
                } else {
                    prepared.tool_warnings.push(Warning::Unsupported {
                        feature: format!("provider-defined tool {}", tool.id),
                        details: Some(format!(
                            "Browser search is only supported on the following models: {}. Current model: {model_id}",
                            get_supported_models_string()
                        )),
                    });
                }
            }
            Tool::Provider(tool) => prepared.tool_warnings.push(Warning::Unsupported {
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
                groq_tools.push(json!({ "type": "function", "function": function }));
            }
        }
    }
    prepared.tools = Some(groq_tools);

    prepared.tool_choice = options.tool_choice.as_ref().map(|choice| match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool { tool_name } => {
            json!({ "type": "function", "function": { "name": tool_name } })
        }
    });
    prepared
}
