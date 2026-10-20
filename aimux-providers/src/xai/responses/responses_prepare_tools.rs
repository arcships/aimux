//! Tools and tool choice for the Responses API (`xai-responses-prepare-tools.ts`).

use aimux_core::error::AiMuxError;
use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::Tool;
use aimux_core::types::Warning;
use serde_json::{Value, json};

use super::responses_language_model_options::{string_array, validate_value};

#[derive(Debug, Clone)]
pub struct PreparedResponsesTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
    /// Names of provider-executed tools, keyed by tool id (e.g. "xai.web_search").
    pub provider_tool_names: std::collections::HashMap<String, String>,
}

/// # Errors
/// Rejects provider-tool arguments that do not match the upstream schemas.
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
) -> Result<PreparedResponsesTools, AiMuxError> {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    for tool in non_empty.into_iter().flatten() {
        if let Tool::Provider(tool) = tool {
            validate_provider_tool(tool)?;
        }
    }
    let mut tool_warnings: Vec<Warning> = Vec::new();
    let mut provider_tool_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let xai_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => {
                        let mut func = json!({
                            "type": "function",
                            "name": ft.name,
                            "parameters": ft.input_schema,
                        });
                        if let Some(ref desc) = ft.description {
                            func["description"] = json!(desc);
                        }
                        if let Some(strict) = ft.strict {
                            func["strict"] = json!(strict);
                        }
                        Some(func)
                    }
                    Tool::Provider(pt) => {
                        provider_tool_names.insert(pt.id.to_string(), pt.name.clone());
                        prepare_provider_tool(pt, &mut tool_warnings)
                    }
                })
                .collect();
            Some(xai_tools)
        }
    };

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => {
            // Build a map of tool name -> is_provider_tool from the original tools.
            let non_empty_tools = non_empty.unwrap();
            match tc {
                LanguageModelV4ToolChoice::Auto => Some(json!("auto")),
                LanguageModelV4ToolChoice::None => Some(json!("none")),
                LanguageModelV4ToolChoice::Required => Some(json!("required")),
                LanguageModelV4ToolChoice::Tool { tool_name } => {
                    let selected = non_empty_tools.iter().rev().find(|t| match t {
                        Tool::Function(ft) => ft.name == *tool_name,
                        Tool::Provider(pt) => pt.name == *tool_name,
                    });

                    match selected {
                        None => None,
                        Some(Tool::Function(_)) => {
                            Some(json!({ "type": "function", "name": tool_name }))
                        }
                        Some(Tool::Provider(pt)) => {
                            // Server-side tools cannot be forced via toolChoice.
                            tool_warnings.push(Warning::Unsupported {
                                feature: format!("toolChoice for server-side tool \"{}\"", pt.name),
                                details: None,
                            });
                            None
                        }
                    }
                }
            }
        }
    };

    Ok(PreparedResponsesTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
        provider_tool_names,
    })
}

fn prepare_provider_tool(
    pt: &aimux_core::tool::ProviderTool,
    warnings: &mut Vec<Warning>,
) -> Option<Value> {
    let args = &pt.args;
    match pt.id.as_str() {
        "xai.web_search" => {
            let mut tool = json!({ "type": "web_search" });
            if let Some(v) = args.get("allowedDomains") {
                tool["allowed_domains"] = v.clone();
            }
            if let Some(v) = args.get("excludedDomains") {
                tool["excluded_domains"] = v.clone();
            }
            if let Some(v) = args.get("enableImageSearch") {
                tool["enable_image_search"] = v.clone();
            }
            if let Some(v) = args.get("enableImageUnderstanding") {
                tool["enable_image_understanding"] = v.clone();
            }
            Some(tool)
        }
        "xai.x_search" => {
            let mut tool = json!({ "type": "x_search" });
            if let Some(v) = args.get("allowedXHandles") {
                tool["allowed_x_handles"] = v.clone();
            }
            if let Some(v) = args.get("excludedXHandles") {
                tool["excluded_x_handles"] = v.clone();
            }
            if let Some(v) = args.get("fromDate") {
                tool["from_date"] = v.clone();
            }
            if let Some(v) = args.get("toDate") {
                tool["to_date"] = v.clone();
            }
            if let Some(v) = args.get("enableImageUnderstanding") {
                tool["enable_image_understanding"] = v.clone();
            }
            if let Some(v) = args.get("enableVideoUnderstanding") {
                tool["enable_video_understanding"] = v.clone();
            }
            Some(tool)
        }
        "xai.code_execution" => Some(json!({ "type": "code_interpreter" })),
        "xai.view_image" => Some(json!({ "type": "view_image" })),
        "xai.view_x_video" => Some(json!({ "type": "view_x_video" })),
        "xai.image_generation" => {
            let mut tool = json!({ "type": "image_generation" });
            if let Some(action) = args.get("action") {
                tool["action"] = action.clone();
            }
            Some(tool)
        }
        "xai.file_search" => {
            let mut tool = json!({ "type": "file_search" });
            if let Some(v) = args.get("vectorStoreIds") {
                tool["vector_store_ids"] = v.clone();
            }
            if let Some(v) = args.get("maxNumResults") {
                tool["max_num_results"] = v.clone();
            }
            Some(tool)
        }
        "xai.mcp" => {
            let mut tool = json!({ "type": "mcp" });
            if let Some(v) = args.get("serverUrl") {
                tool["server_url"] = v.clone();
            }
            if let Some(v) = args.get("serverLabel") {
                tool["server_label"] = v.clone();
            }
            if let Some(v) = args.get("serverDescription") {
                tool["server_description"] = v.clone();
            }
            if let Some(v) = args.get("allowedTools") {
                tool["allowed_tools"] = v.clone();
            }
            if let Some(v) = args.get("headers") {
                tool["headers"] = v.clone();
            }
            if let Some(v) = args.get("authorization") {
                tool["authorization"] = v.clone();
            }
            Some(tool)
        }
        _ => {
            warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", pt.name),
                details: None,
            });
            None
        }
    }
}

fn validate_provider_tool(tool: &aimux_core::tool::ProviderTool) -> Result<(), AiMuxError> {
    let id = tool.id.as_str();
    if ![
        "xai.web_search",
        "xai.x_search",
        "xai.image_generation",
        "xai.file_search",
        "xai.mcp",
    ]
    .contains(&id)
    {
        return Ok(());
    }
    let args = &tool.args;
    for (key, value) in args {
        validate_value(key, value, |value| match (id, key.as_str()) {
            ("xai.web_search", "allowedDomains" | "excludedDomains") => string_array(value, 5),
            ("xai.x_search", "allowedXHandles" | "excludedXHandles") => string_array(value, 10),
            ("xai.web_search", "enableImageSearch" | "enableImageUnderstanding")
            | ("xai.x_search", "enableImageUnderstanding" | "enableVideoUnderstanding") => {
                value.is_boolean()
            }
            ("xai.x_search", "fromDate" | "toDate") => value.is_string(),
            ("xai.image_generation", "action") => value
                .as_str()
                .is_some_and(|v| ["auto", "generate", "edit"].contains(&v)),
            ("xai.file_search", "vectorStoreIds") | ("xai.mcp", "allowedTools") => {
                string_array(value, usize::MAX)
            }
            ("xai.file_search", "maxNumResults") => value.is_number(),
            ("xai.mcp", "serverUrl" | "serverLabel" | "serverDescription" | "authorization") => {
                value.is_string()
            }
            ("xai.mcp", "headers") => value
                .as_object()
                .is_some_and(|headers| headers.values().all(Value::is_string)),
            _ => true,
        })?;
    }
    for key in match id {
        "xai.file_search" => &["vectorStoreIds"][..],
        "xai.mcp" => &["serverUrl"][..],
        _ => &[],
    } {
        if !args.contains_key(*key) {
            return Err(AiMuxError::InvalidArgument(format!(
                "Missing {key} for {}",
                tool.name
            )));
        }
    }
    Ok(())
}
