//! Tool preparation for the Converse API (amazon-bedrock-prepare-tools.ts), with the
//! provider-defined tool input schemas of the pinned upstream factories.

use super::anthropic_model_support::supports_strict_tools;
use crate::anthropic::internal::{AnthropicTool, prepare_tools_with_provider};
use aimux_core::AiMuxError;
use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::Warning;
use serde_json::{Value, json};

fn input_schema(id: &str) -> Option<Value> {
    let schema = match id {
        "anthropic.advisor_20260301" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{},"additionalProperties":false}"#
        }
        "anthropic.bash_20241022" | "anthropic.bash_20250124" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"command":{"type":"string"},"restart":{"type":"boolean"}},"required":["command"],"additionalProperties":false}"#
        }
        "anthropic.code_execution_20250522" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"code":{"type":"string"}},"required":["code"],"additionalProperties":false}"#
        }
        "anthropic.code_execution_20250825" | "anthropic.code_execution_20260120" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","oneOf":[{"type":"object","properties":{"type":{"type":"string","const":"programmatic-tool-call"},"code":{"type":"string"}},"required":["type","code"],"additionalProperties":false},{"type":"object","properties":{"type":{"type":"string","const":"bash_code_execution"},"command":{"type":"string"}},"required":["type","command"],"additionalProperties":false},{"oneOf":[{"type":"object","properties":{"type":{"type":"string","const":"text_editor_code_execution"},"command":{"type":"string","const":"view"},"path":{"type":"string"}},"required":["type","command","path"],"additionalProperties":false},{"type":"object","properties":{"type":{"type":"string","const":"text_editor_code_execution"},"command":{"type":"string","const":"create"},"path":{"type":"string"},"file_text":{"type":["string","null"]}},"required":["type","command","path"],"additionalProperties":false},{"type":"object","properties":{"type":{"type":"string","const":"text_editor_code_execution"},"command":{"type":"string","const":"str_replace"},"path":{"type":"string"},"old_str":{"type":"string"},"new_str":{"type":"string"}},"required":["type","command","path","old_str","new_str"],"additionalProperties":false}]}]}"#
        }
        "anthropic.computer_20241022" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"action":{"type":"string","enum":["key","type","mouse_move","left_click","left_click_drag","right_click","middle_click","double_click","screenshot","cursor_position"]},"coordinate":{"type":"array","items":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}},"text":{"type":"string"}},"required":["action"],"additionalProperties":false}"#
        }
        "anthropic.computer_20250124" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"action":{"type":"string","enum":["key","hold_key","type","cursor_position","mouse_move","left_mouse_down","left_mouse_up","left_click","left_click_drag","right_click","middle_click","double_click","triple_click","scroll","wait","screenshot"]},"coordinate":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":2,"maxItems":2},"duration":{"type":"number"},"scroll_amount":{"type":"number"},"scroll_direction":{"type":"string","enum":["up","down","left","right"]},"start_coordinate":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":2,"maxItems":2},"text":{"type":"string"}},"required":["action"],"additionalProperties":false}"#
        }
        "anthropic.computer_20251124" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"action":{"type":"string","enum":["key","hold_key","type","cursor_position","mouse_move","left_mouse_down","left_mouse_up","left_click","left_click_drag","right_click","middle_click","double_click","triple_click","scroll","wait","screenshot","zoom"]},"coordinate":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":2,"maxItems":2},"duration":{"type":"number"},"region":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":4,"maxItems":4},"scroll_amount":{"type":"number"},"scroll_direction":{"type":"string","enum":["up","down","left","right"]},"start_coordinate":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":2,"maxItems":2},"text":{"type":"string"}},"required":["action"],"additionalProperties":false}"#
        }
        "anthropic.computer_toolset_20260801" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"action":{"type":"string","enum":["screenshot","zoom","left_click","right_click","middle_click","double_click","triple_click","left_click_drag","mouse_move","left_mouse_down","left_mouse_up","cursor_position","scroll","type","key","hold_key","wait"]},"coordinate":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":2,"maxItems":2},"duration":{"type":"number"},"region":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":4,"maxItems":4},"repeat":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},"scroll_amount":{"type":"number"},"scroll_direction":{"type":"string","enum":["up","down","left","right"]},"start_coordinate":{"type":"array","items":[{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}],"additionalItems":false,"minItems":2,"maxItems":2},"text":{"type":"string"}},"required":["action"],"additionalProperties":false}"#
        }
        "anthropic.memory_20250818" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","oneOf":[{"type":"object","properties":{"command":{"type":"string","const":"view"},"path":{"type":"string"},"view_range":{"type":"array","items":[{"type":"number"},{"type":"number"}],"additionalItems":false,"minItems":2,"maxItems":2}},"required":["command","path"],"additionalProperties":false},{"type":"object","properties":{"command":{"type":"string","const":"create"},"path":{"type":"string"},"file_text":{"type":"string"}},"required":["command","path","file_text"],"additionalProperties":false},{"type":"object","properties":{"command":{"type":"string","const":"str_replace"},"path":{"type":"string"},"old_str":{"type":"string"},"new_str":{"type":"string"}},"required":["command","path","old_str","new_str"],"additionalProperties":false},{"type":"object","properties":{"command":{"type":"string","const":"insert"},"path":{"type":"string"},"insert_line":{"type":"number"},"insert_text":{"type":"string"}},"required":["command","path","insert_line","insert_text"],"additionalProperties":false},{"type":"object","properties":{"command":{"type":"string","const":"delete"},"path":{"type":"string"}},"required":["command","path"],"additionalProperties":false},{"type":"object","properties":{"command":{"type":"string","const":"rename"},"old_path":{"type":"string"},"new_path":{"type":"string"}},"required":["command","old_path","new_path"],"additionalProperties":false}]}"#
        }
        "anthropic.text_editor_20241022" | "anthropic.text_editor_20250124" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"command":{"type":"string","enum":["view","create","str_replace","insert","undo_edit"]},"path":{"type":"string"},"file_text":{"type":"string"},"insert_line":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},"new_str":{"type":"string"},"insert_text":{"type":"string"},"old_str":{"type":"string"},"view_range":{"type":"array","items":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}}},"required":["command","path"],"additionalProperties":false}"#
        }
        "anthropic.text_editor_20250429" | "anthropic.text_editor_20250728" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"command":{"type":"string","enum":["view","create","str_replace","insert"]},"path":{"type":"string"},"file_text":{"type":"string"},"insert_line":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991},"new_str":{"type":"string"},"insert_text":{"type":"string"},"old_str":{"type":"string"},"view_range":{"type":"array","items":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}}},"required":["command","path"],"additionalProperties":false}"#
        }
        "anthropic.web_fetch_20250910"
        | "anthropic.web_fetch_20260209"
        | "anthropic.web_fetch_20260318" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"url":{"type":"string"}},"required":["url"],"additionalProperties":false}"#
        }
        "anthropic.web_search_20250305"
        | "anthropic.web_search_20260209"
        | "anthropic.web_search_20260318" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}"#
        }
        "anthropic.tool_search_regex_20251119" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"pattern":{"type":"string"},"limit":{"type":"number"}},"required":["pattern"],"additionalProperties":false}"#
        }
        "anthropic.tool_search_bm25_20251119" => {
            r#"{"$schema":"http://json-schema.org/draft-07/schema#","type":"object","properties":{"query":{"type":"string"},"limit":{"type":"number"}},"required":["query"],"additionalProperties":false}"#
        }
        _ => return None,
    };
    Some(serde_json::from_str(schema).expect("upstream tool schema"))
}

pub(crate) struct PreparedProviderTools {
    pub tools: Vec<Value>,
    pub tool_choice: Option<Value>,
    pub betas: Vec<String>,
    pub warnings: Vec<Warning>,
    pub using_anthropic_tools: bool,
}

pub(crate) fn prepare_provider_tools(
    tools: Option<&[Tool]>,
    choice: Option<&LanguageModelV4ToolChoice>,
    anthropic: bool,
    disable_parallel: bool,
    rejects_forced: bool,
) -> Result<PreparedProviderTools, AiMuxError> {
    let mut result = PreparedProviderTools {
        tools: Vec::new(),
        tool_choice: None,
        betas: Vec::new(),
        warnings: Vec::new(),
        using_anthropic_tools: false,
    };
    let providers: Vec<_> = tools
        .unwrap_or_default()
        .iter()
        .filter_map(|tool| {
            let Tool::Provider(tool) = tool else {
                return None;
            };
            if matches!(
                tool.id.as_str(),
                "anthropic.web_search_20250305"
                    | "anthropic.web_search_20260318"
                    | "anthropic.web_fetch_20260318"
            ) {
                let kind = tool.id.as_str().strip_prefix("anthropic.").unwrap();
                result.warnings.push(Warning::Unsupported {
                    feature: format!("{kind} tool"),
                    details: Some(format!(
                        "The {kind} tool is not supported on Amazon Bedrock."
                    )),
                });
                None
            } else {
                Some(tool)
            }
        })
        .collect();
    let selected: Vec<_> = providers
        .iter()
        .copied()
        .filter(|tool| {
            !anthropic
                || !rejects_forced
                || !matches!(choice, Some(LanguageModelV4ToolChoice::Tool { tool_name }) if tool_name != &tool.name)
        })
        .collect();
    if !anthropic || selected.is_empty() {
        for tool in selected {
            result.warnings.push(Warning::Unsupported {
                feature: format!("tool {}", tool.id),
                details: None,
            });
        }
        return Ok(result);
    }
    result.using_anthropic_tools = true;
    let anthropic_tools: Vec<_> = providers
        .iter()
        .map(|tool| AnthropicTool::Provider {
            id: tool.id.to_string(),
            name: tool.name.clone(),
            args: tool.args.clone(),
        })
        .collect();
    let fallback = rejects_forced
        && matches!(
            choice,
            Some(LanguageModelV4ToolChoice::Required | LanguageModelV4ToolChoice::Tool { .. })
        );
    let prepared = prepare_tools_with_provider(
        Some(&anthropic_tools),
        if fallback {
            Some(&LanguageModelV4ToolChoice::Auto)
        } else {
            choice
        },
        disable_parallel,
        false,
        false,
        false,
    )?;
    result.tool_choice = prepared.tool_choice;
    result.betas.extend(prepared.betas);
    result.warnings.extend(prepared.tool_warnings);
    if fallback {
        let details = match choice {
            Some(LanguageModelV4ToolChoice::Tool { tool_name }) => format!("toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the '{tool_name}' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made."),
            _ => "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.".to_owned(),
        };
        result.warnings.push(Warning::Unsupported {
            feature: "toolChoice".into(),
            details: Some(details),
        });
    }
    for tool in selected {
        if let Some(schema) = input_schema(tool.id.as_str()) {
            result
                .tools
                .push(json!({"toolSpec":{"name":tool.name,"inputSchema":{"json":schema}}}));
        } else {
            result.warnings.push(Warning::Unsupported {
                feature: "tool ${tool.id}".into(),
                details: None,
            });
        }
    }
    Ok(result)
}

pub(super) fn is_strict_schema_compatible(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return true;
    };
    let is_object = schema["type"] == "object"
        || schema["type"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "object"));
    if is_object && schema["additionalProperties"] != false {
        return false;
    }
    for key in [
        "properties",
        "patternProperties",
        "definitions",
        "$defs",
        "dependencies",
    ] {
        if let Some(map) = object.get(key).and_then(Value::as_object)
            && map.values().any(|v| !is_strict_schema_compatible(v))
        {
            return false;
        }
    }
    for key in [
        "propertyNames",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "items",
        "anyOf",
        "allOf",
        "oneOf",
    ] {
        if let Some(value) = object.get(key) {
            if let Some(array) = value.as_array() {
                if array.iter().any(|v| !is_strict_schema_compatible(v)) {
                    return false;
                }
            } else if !is_strict_schema_compatible(value) {
                return false;
            }
        }
    }
    true
}

/// Prepare `FunctionTool`s into the Bedrock `toolConfig` JSON shape.
///
/// Mirrors the function-tool subset of the TS `prepareTools`:
/// - no tools (or `toolChoice: none`) → empty `toolConfig` `{}`
/// - `toolChoice: tool` filters tools to the named one
/// - `toolChoice: auto/required/tool` → `{ auto: {} }` / `{ any: {} }` /
///   `{ tool: { name } }`
/// - `description` is omitted when empty/whitespace
/// - `strict` is passed through only for models that `supports_strict_tools`
///
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
    model_id: &str,
) -> Value {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    let Some(tools) = non_empty else {
        return json!({});
    };

    // `toolChoice: none` clears tools entirely (matches TS).
    if matches!(tool_choice, Some(LanguageModelV4ToolChoice::None)) {
        return json!({});
    }

    // `toolChoice: tool` filters function tools to the named one.
    let filtered: Vec<&FunctionTool> = match tool_choice {
        Some(LanguageModelV4ToolChoice::Tool { tool_name }) => {
            tools.iter().filter(|t| &t.name == tool_name).collect()
        }
        _ => tools.iter().collect(),
    };

    let supports_strict = supports_strict_tools(model_id);
    let tool_specs: Vec<Value> = filtered
        .iter()
        .map(|t| {
            let mut spec = serde_json::Map::new();
            spec.insert("name".to_string(), json!(t.name));
            if let Some(ref desc) = t.description
                && !desc.trim().is_empty()
            {
                spec.insert("description".to_string(), json!(desc));
            }
            if let Some(strict) = t.strict
                && supports_strict
                && (!strict || is_strict_schema_compatible(&t.input_schema))
            {
                spec.insert("strict".to_string(), json!(strict));
            }
            spec.insert("inputSchema".to_string(), json!({ "json": t.input_schema }));
            json!({ "toolSpec": Value::Object(spec) })
        })
        .collect();

    if tool_specs.is_empty() {
        return json!({});
    }

    let tool_choice_val = match tool_choice {
        Some(LanguageModelV4ToolChoice::Auto) => json!({ "auto": {} }),
        Some(LanguageModelV4ToolChoice::Required) => json!({ "any": {} }),
        Some(LanguageModelV4ToolChoice::Tool { tool_name }) => {
            json!({ "tool": { "name": tool_name } })
        }
        Some(LanguageModelV4ToolChoice::None) => unreachable!(),
        None => return json!({ "tools": tool_specs }),
    };

    json!({ "tools": tool_specs, "toolChoice": tool_choice_val })
}
