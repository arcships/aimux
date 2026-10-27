//! Tools and tool choice for the Responses API (`huggingface-responses-prepare-tools.ts`).

use serde_json::{Map, Value, json};

use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::Tool;
use aimux_core::types::Warning;

/// Prepare tools and tool choice for the Hugging Face Responses API.
///
/// Returns `(tools, tool_choice, warnings)`. Only function tools are supported;
/// provider-defined tools produce a warning.
#[must_use]
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
) -> (Option<Vec<Value>>, Option<Value>, Vec<Warning>) {
    let tools = match tools {
        Some(t) if !t.is_empty() => t,
        _ => return (None, None, vec![]),
    };

    let mut hf_tools: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for tool in tools {
        match tool {
            Tool::Function(ft) => {
                let mut t = Map::new();
                t.insert("type".into(), json!("function"));
                t.insert("name".into(), json!(ft.name));
                if let Some(desc) = &ft.description {
                    t.insert("description".into(), json!(desc));
                }
                t.insert("parameters".into(), ft.input_schema.clone());
                hf_tools.push(Value::Object(t));
            }
            Tool::Provider(pt) => {
                warnings.push(Warning::Unsupported {
                    feature: format!("provider-defined tool {}", pt.id),
                    details: None,
                });
            }
        }
    }

    let mapped_tool_choice = match tool_choice {
        Some(LanguageModelV4ToolChoice::Auto) => Some(json!("auto")),
        Some(LanguageModelV4ToolChoice::Required) => Some(json!("required")),
        None | Some(LanguageModelV4ToolChoice::None) => None, // not supported, ignore
        Some(LanguageModelV4ToolChoice::Tool { tool_name }) => Some(json!({
            "type": "function",
            "function": { "name": tool_name }
        })),
    };

    (Some(hf_tools), mapped_tool_choice, warnings)
}
