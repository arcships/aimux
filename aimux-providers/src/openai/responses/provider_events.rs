//! Provider tool output mapping from the pinned Responses implementation.

use std::collections::HashMap;

use aimux_core::result::{GenerateContent, RawToolApprovalRequest};
use aimux_core::shared::provider_namespace;
use aimux_core::tool::{RawToolCall, ToolResult};
use serde_json::{Value, json};

pub(crate) fn provider_tool_content(
    part: &Value,
    provider_key: &str,
    tool_names: &HashMap<String, String>,
    shell_provider_executed: bool,
    mcp_tool_call_id: Option<&str>,
) -> Vec<GenerateContent> {
    let kind = part["type"].as_str().unwrap_or_default();
    let item_id = part["id"].as_str().unwrap_or_default();
    let call_id = part["call_id"].as_str().unwrap_or(item_id);
    let metadata = || {
        Some(
            provider_namespace(provider_key, json!({"itemId": item_id}))
                .expect("provider metadata must be an object"),
        )
    };
    let name = |canonical: &str| {
        tool_names
            .get(canonical)
            .cloned()
            .unwrap_or_else(|| canonical.to_string())
    };
    let call = |id: &str, tool_name: String, input: String, executed, dynamic, meta| {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id: id.to_string(),
            tool_name,
            input,
            provider_executed: executed,
            dynamic,
            provider_metadata: meta,
        })
    };
    let result = |id: &str, tool_name: String, value: Value, meta| {
        GenerateContent::ToolResult(ToolResult {
            tool_call_id: id.to_string(),
            tool_name,
            result: value,
            is_error: None,
            preliminary: None,
            dynamic: None,
            provider_metadata: meta,
        })
    };
    let pair = |canonical: &str, input: String, output: Value| {
        vec![
            call(item_id, name(canonical), input, Some(true), None, None),
            result(item_id, name(canonical), output, None),
        ]
    };
    match kind {
        "image_generation_call" => pair(
            "image_generation",
            "{}".into(),
            json!({"result": part["result"]}),
        ),
        "web_search_call" => pair(
            "web_search",
            "{}".into(),
            web_search_output(&part["action"]),
        ),
        "file_search_call" => pair(
            "file_search",
            "{}".into(),
            json!({
                "queries": part["queries"],
                "results": part["results"].as_array().map(|results| results.iter().map(|value| {
                    json!({"attributes": value["attributes"], "fileId": value["file_id"],
                        "filename": value["filename"], "score": value["score"], "text": value["text"]})
                }).collect::<Vec<_>>()),
            }),
        ),
        "code_interpreter_call" => pair(
            "code_interpreter",
            json!({"code": part["code"], "containerId": part["container_id"]}).to_string(),
            json!({"outputs": part["outputs"]}),
        ),
        "computer_call" if part["call_id"].is_null() => pair(
            "computer_use",
            String::new(),
            json!({"type": "computer_use_tool_result", "status": part["status"]}),
        ),
        "computer_call" => vec![call(
            call_id,
            name("computer"),
            computer_input(part).to_string(),
            None,
            None,
            metadata(),
        )],
        "local_shell_call" => vec![call(
            call_id,
            name("local_shell"),
            json!({"action": part["action"]}).to_string(),
            None,
            None,
            metadata(),
        )],
        "shell_call" => vec![call(
            call_id,
            name("shell"),
            json!({"action": {"commands": part["action"]["commands"]}}).to_string(),
            shell_provider_executed.then_some(true),
            None,
            metadata(),
        )],
        "shell_call_output" => vec![result(
            call_id,
            name("shell"),
            json!({"output": part["output"].as_array().map(|items| items.iter().map(|item| {
                let outcome = if item["outcome"]["type"] == "exit" {
                    json!({"type": "exit", "exitCode": item["outcome"]["exit_code"]})
                } else {
                    json!({"type": "timeout"})
                };
                json!({"stdout": item["stdout"], "stderr": item["stderr"], "outcome": outcome})
            }).collect::<Vec<_>>())}),
            None,
        )],
        "apply_patch_call" => vec![call(
            call_id,
            name("apply_patch"),
            json!({"callId": part["call_id"], "operation": part["operation"]}).to_string(),
            None,
            None,
            metadata(),
        )],
        "program" => vec![call(
            call_id,
            name("programmatic_tool_calling"),
            json!({"code": part["code"], "fingerprint": part["fingerprint"]}).to_string(),
            Some(true),
            None,
            metadata(),
        )],
        "program_output" => vec![result(
            call_id,
            name("programmatic_tool_calling"),
            json!({"result": part["result"], "status": part["status"]}),
            metadata(),
        )],
        "tool_search_call" => {
            let mut input = json!({});
            for field in ["arguments", "call_id"] {
                if let Some(value) = part.get(field) {
                    input[field] = value.clone();
                }
            }
            vec![call(
                call_id,
                name("tool_search"),
                input.to_string(),
                (part["execution"] == "server").then_some(true),
                None,
                metadata(),
            )]
        }
        "tool_search_output" => vec![result(
            call_id,
            name("tool_search"),
            json!({"tools": part["tools"]}),
            metadata(),
        )],
        "compaction" => vec![GenerateContent::Custom {
            kind: "openai.compaction".into(),
            provider_metadata: Some(
                provider_namespace(
                    provider_key,
                    json!({
                        "type": "compaction", "itemId": part["id"],
                        "encryptedContent": part["encrypted_content"],
                    }),
                )
                .expect("provider metadata must be an object"),
            ),
        }],
        "mcp_approval_request" => {
            let tool_call_id = aimux_provider_utils::generate_id();
            let approval_id = part
                .get("approval_request_id")
                .and_then(Value::as_str)
                .unwrap_or(item_id)
                .to_owned();
            vec![
                call(
                    &tool_call_id,
                    format!("mcp.{}", part["name"].as_str().unwrap_or_default()),
                    part["arguments"].as_str().unwrap_or_default().to_owned(),
                    Some(true),
                    Some(true),
                    None,
                ),
                GenerateContent::ToolApprovalRequest(RawToolApprovalRequest {
                    approval_id,
                    tool_call_id,
                    provider_metadata: None,
                }),
            ]
        }
        "mcp_call" => {
            let tool_name = format!("mcp.{}", part["name"].as_str().unwrap_or_default());
            let mut output = json!({"type": "call", "serverLabel": part["server_label"],
                "name": part["name"], "arguments": part["arguments"]});
            for field in ["output", "error"] {
                if let Some(value) = part.get(field).filter(|value| !value.is_null()) {
                    output[field] = value.clone();
                }
            }
            vec![
                call(
                    mcp_tool_call_id.unwrap_or(item_id),
                    tool_name.clone(),
                    part["arguments"].as_str().unwrap_or_default().to_string(),
                    Some(true),
                    Some(true),
                    None,
                ),
                result(
                    mcp_tool_call_id.unwrap_or(item_id),
                    tool_name,
                    output,
                    metadata(),
                ),
            ]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn web_search_output(action: &Value) -> Value {
    match action["type"].as_str() {
        Some("search") => {
            let mut output = json!({"action": {"type": "search"}});
            for field in ["query", "queries"] {
                if let Some(value) = action.get(field).filter(|value| !value.is_null()) {
                    output["action"][field] = value.clone();
                }
            }
            if let Some(sources) = action.get("sources").filter(|value| !value.is_null()) {
                output["sources"] = sources.clone();
            }
            output
        }
        Some("open_page" | "find_in_page") => {
            let kind = if action["type"] == "open_page" {
                "openPage"
            } else {
                "findInPage"
            };
            let mut output = json!({"action": {"type": kind}});
            for field in ["url", "pattern"] {
                if field == "pattern" && kind == "openPage" {
                    continue;
                }
                if let Some(value) = action.get(field) {
                    output["action"][field] = value.clone();
                }
            }
            output
        }
        _ => json!({}),
    }
}

fn computer_input(part: &Value) -> Value {
    let actions = part["actions"].as_array().cloned().unwrap_or_else(|| {
        part.get("action")
            .filter(|action| !action.is_null())
            .cloned()
            .into_iter()
            .collect()
    });
    let actions: Vec<_> = actions
        .into_iter()
        .map(|mut action| {
            if action["type"] == "scroll"
                && let Some(object) = action.as_object_mut()
            {
                for (from, to) in [("scroll_x", "scrollX"), ("scroll_y", "scrollY")] {
                    if let Some(value) = object.remove(from) {
                        object.insert(to.to_string(), value);
                    }
                }
            }
            action
        })
        .collect();
    let checks: Vec<_> = part["pending_safety_checks"]
        .as_array()
        .map(|checks| {
            checks
                .iter()
                .map(|check| {
                    let mut result = json!({"id": check["id"]});
                    for field in ["code", "message"] {
                        if let Some(value) = check.get(field).filter(|value| !value.is_null()) {
                            result[field] = value.clone();
                        }
                    }
                    result
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"actions": actions, "pendingSafetyChecks": checks, "status": part["status"]})
}
