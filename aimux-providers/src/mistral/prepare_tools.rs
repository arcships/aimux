//! `mistral-prepare-tools.ts`.

use serde_json::{Value, json};

use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::FunctionTool;

// ── Prepared tools ──────────────────────────────────────────────────────────

/// The result of preparing tools for a Mistral request body.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<String>,
}

/// Prepare `FunctionTool`s into the Mistral `tools` / `tool_choice` JSON shape.
///
/// Key difference from OpenAI: `LanguageModelV4ToolChoice::Required` maps to `"any"` (not
/// `"required"`), and `LanguageModelV4ToolChoice::Tool` filters the tools array and also uses
/// `"any"`.
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
) -> PreparedTools {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());

    let tool_warnings: Vec<String> = Vec::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mistral_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let mut func = json!({
                        "name": t.name,
                        "parameters": t.input_schema,
                    });
                    if let Some(ref desc) = t.description {
                        func["description"] = json!(desc);
                    }
                    if let Some(strict) = t.strict {
                        func["strict"] = json!(strict);
                    }
                    json!({ "type": "function", "function": func })
                })
                .collect();
            Some(mistral_tools)
        }
    };

    // Handle LanguageModelV4ToolChoice::Tool which needs to filter the tools array.
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
            tool_choice: Some(json!("any")),
            tool_warnings,
        };
    }

    let tool_choice_opt = match (&tools_opt, tool_choice) {
        (None, _) => None,
        (Some(_), None) => None,
        (Some(_), Some(tc)) => match tc {
            LanguageModelV4ToolChoice::Auto => Some(json!("auto")),
            LanguageModelV4ToolChoice::None => Some(json!("none")),
            LanguageModelV4ToolChoice::Required => Some(json!("any")),
            LanguageModelV4ToolChoice::Tool { .. } => Some(json!("any")),
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
