//! Provider-defined tool input schemas from the pinned upstream factories.

use serde_json::Value;

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

use aimux_core::AiMuxError;

use aimux_core::tool::{Tool, ToolChoice};
use aimux_core::types::Warning;
use serde_json::json;

use crate::anthropic::prepare_tools::{AnthropicTool, prepare_tools_with_provider};

pub(crate) struct PreparedProviderTools {
    pub tools: Vec<Value>,
    pub tool_choice: Option<Value>,
    pub betas: Vec<String>,
    pub warnings: Vec<Warning>,
    pub using_anthropic_tools: bool,
}

pub(crate) fn prepare_provider_tools(
    tools: Option<&[Tool]>,
    choice: &ToolChoice,
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
                let kind = tool.id.strip_prefix("anthropic.").unwrap();
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
                || !matches!(choice, ToolChoice::Tool { tool_name } if tool_name != &tool.name)
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
            id: tool.id.clone(),
            name: tool.name.clone(),
            args: tool.args.clone(),
        })
        .collect();
    let fallback =
        rejects_forced && matches!(choice, ToolChoice::Required | ToolChoice::Tool { .. });
    let prepared = prepare_tools_with_provider(
        Some(&anthropic_tools),
        Some(if fallback { &ToolChoice::Auto } else { choice }),
        disable_parallel,
        false,
        false,
        false,
    );
    result.tool_choice = prepared.tool_choice;
    for tool in &providers {
        validate_args(&tool.id, &tool.args)?;
    }
    result.betas.extend(prepared.betas);
    result.warnings.extend(prepared.tool_warnings);
    if fallback {
        let details = match choice {
            ToolChoice::Tool { tool_name } => format!("toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the '{tool_name}' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made."),
            _ => "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.".to_owned(),
        };
        result.warnings.push(Warning::Unsupported {
            feature: "toolChoice".into(),
            details: Some(details),
        });
    }
    for tool in selected {
        if let Some(schema) = input_schema(&tool.id) {
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

// Upstream validates configuration only for these factories; older tools pass
// their configuration through unchecked.
fn validate_args(id: &str, args: &Value) -> Result<(), AiMuxError> {
    let web_fetch = matches!(
        id,
        "anthropic.web_fetch_20250910" | "anthropic.web_fetch_20260209"
    );
    let web_search = id == "anthropic.web_search_20260209";
    if !web_fetch
        && !web_search
        && !matches!(
            id,
            "anthropic.advisor_20260301"
                | "anthropic.computer_toolset_20260801"
                | "anthropic.text_editor_20250728"
        )
    {
        return Ok(());
    }
    let invalid = || {
        AiMuxError::InvalidArgument(format!(
            "Invalid configuration for provider-defined tool {id}"
        ))
    };
    let args = args.as_object().ok_or_else(invalid)?;
    let optional = |name: &str, valid: fn(&Value) -> bool| args.get(name).is_none_or(valid);
    let valid = if web_fetch || web_search {
        optional("maxUses", Value::is_number)
            && optional("allowedDomains", |v| {
                v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
            })
            && optional("blockedDomains", |v| {
                v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
            })
            && if web_fetch {
                optional("maxContentTokens", Value::is_number)
                    && optional("citations", |v| {
                        v.is_object() && v.get("enabled").is_some_and(Value::is_boolean)
                    })
            } else {
                optional("userLocation", |v| {
                    v.is_object()
                        && v.get("type").and_then(Value::as_str) == Some("approximate")
                        && ["city", "region", "country", "timezone"]
                            .iter()
                            .all(|key| v.get(*key).is_none_or(Value::is_string))
                })
            }
    } else if id == "anthropic.advisor_20260301" {
        args.get("model").is_some_and(Value::is_string)
            && optional("maxUses", Value::is_number)
            && optional("maxTokens", |v| {
                v.as_f64()
                    .is_some_and(|n| (1024.0..=9007199254740991.0).contains(&n) && n.fract() == 0.0)
            })
            && optional("caching", |v| {
                v.is_object()
                    && v.get("type").and_then(Value::as_str) == Some("ephemeral")
                    && matches!(v.get("ttl").and_then(Value::as_str), Some("5m" | "1h"))
            })
    } else if id == "anthropic.text_editor_20250728" {
        optional("maxCharacters", Value::is_number)
    } else {
        optional("configs", |v| {
            v.as_object().is_some_and(|configs| {
                configs.iter().all(|(member, config)| {
                    matches!(
                        member.as_str(),
                        "screenshot"
                            | "zoom"
                            | "left_click"
                            | "right_click"
                            | "middle_click"
                            | "double_click"
                            | "triple_click"
                            | "left_click_drag"
                            | "mouse_move"
                            | "left_mouse_down"
                            | "left_mouse_up"
                            | "cursor_position"
                            | "scroll"
                            | "type"
                            | "key"
                            | "hold_key"
                            | "wait"
                    ) && config.is_object()
                        && ["enabled", "deferLoading"]
                            .iter()
                            .all(|key| config.get(*key).is_none_or(Value::is_boolean))
                })
            })
        })
    };
    if valid { Ok(()) } else { Err(invalid()) }
}
