//! `cohere-prepare-tools.ts`.

use serde_json::{Value, json};

use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::Tool;
use aimux_core::types::Warning;

// ── Prepared tools ──────────────────────────────────────────────────────────

/// The result of preparing tools for a Cohere request body.
#[derive(Debug, Clone)]
pub struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

/// Prepare `Tool`s into the Cohere `tools` / `tool_choice` JSON shape.
///
/// Mirrors the TS `cohere-prepare-tools.ts`. Provider-defined tools are not
/// supported by Cohere and emit an `Unsupported` warning (the tool is dropped).
///
/// Key differences from OpenAI:
/// - `LanguageModelV4ToolChoice::Auto` → omit `tool_choice` (Cohere default)
/// - `LanguageModelV4ToolChoice::None` → `"NONE"`
/// - `LanguageModelV4ToolChoice::Required` → `"REQUIRED"`
/// - `LanguageModelV4ToolChoice::Tool` → filter tools and use `"REQUIRED"`
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
) -> PreparedTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());

    let mut tool_warnings: Vec<Warning> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mut cohere_tools: Vec<Value> = Vec::new();
            for t in tools {
                match t {
                    Tool::Function(ft) => {
                        let mut func = json!({
                            "name": ft.name,
                            "parameters": ft.input_schema,
                        });
                        if let Some(ref desc) = ft.description {
                            func["description"] = json!(desc);
                        }
                        cohere_tools.push(json!({ "type": "function", "function": func }));
                    }
                    Tool::Provider(pt) => {
                        // Provider-defined tools are not supported by Cohere.
                        tool_warnings.push(Warning::Unsupported {
                            feature: format!("provider-defined tool {}", pt.id),
                            details: None,
                        });
                    }
                }
            }
            Some(cohere_tools)
        }
    };

    // Handle LanguageModelV4ToolChoice::Tool which filters tools.
    if let (Some(tools), Some(LanguageModelV4ToolChoice::Tool { tool_name })) =
        (&tools_opt, tool_choice)
    {
        let filtered: Vec<Value> = tools
            .iter()
            .filter(|t| t["function"]["name"].as_str() == Some(tool_name.as_str()))
            .cloned()
            .collect();
        return PreparedTools {
            tools: Some(filtered),
            tool_choice: Some(json!("REQUIRED")),
            tool_warnings,
        };
    }

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => match tc {
            LanguageModelV4ToolChoice::Auto => None,
            LanguageModelV4ToolChoice::None => Some(json!("NONE")),
            LanguageModelV4ToolChoice::Required => Some(json!("REQUIRED")),
            LanguageModelV4ToolChoice::Tool { .. } => Some(json!("REQUIRED")),
        },
    };

    PreparedTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
    }
}

#[cfg(test)]
mod tests;
