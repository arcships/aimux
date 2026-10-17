//! Validation of the pinned provider tool argument schemas.

use aimux_core::AiMuxError;
use serde_json::Value;

fn strings(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
}

fn optional(value: &Value, key: &str, valid: impl Fn(&Value) -> bool) -> bool {
    value.get(key).is_none_or(valid)
}

fn choices(value: &Value, options: &[&str]) -> bool {
    value.as_str().is_some_and(|value| options.contains(&value))
}

fn string_fields(value: &Value, fields: &[&str]) -> bool {
    fields
        .iter()
        .all(|key| optional(value, key, Value::is_string))
}

fn string_record(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|value| value.values().all(Value::is_string))
}

fn filter(value: &Value) -> bool {
    if !value.is_object() {
        return false;
    }
    if choices(&value["type"], &["and", "or"]) {
        value["filters"]
            .as_array()
            .is_some_and(|items| items.iter().all(filter))
    } else {
        value["key"].is_string()
            && choices(
                &value["type"],
                &["eq", "ne", "gt", "gte", "lt", "lte", "in", "nin"],
            )
            && (value["value"].is_string()
                || value["value"].is_number()
                || value["value"].is_boolean()
                || strings(&value["value"]))
    }
}

fn location(value: &Value) -> bool {
    value.is_object()
        && value["type"] == "approximate"
        && string_fields(value, &["country", "city", "region", "timezone"])
}

fn skills(value: &Value) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|skill| {
            skill.is_object()
                && match skill["type"].as_str() {
                    Some("skillReference") => {
                        string_record(&skill["providerReference"])
                            && optional(skill, "version", Value::is_string)
                    }
                    Some("inline") => {
                        skill["name"].is_string()
                            && skill["description"].is_string()
                            && skill["source"].is_object()
                            && skill["source"]["type"] == "base64"
                            && skill["source"]["mediaType"] == "application/zip"
                            && skill["source"]["data"].is_string()
                    }
                    _ => false,
                }
        })
    })
}

fn environment(value: &Value) -> bool {
    if !value.is_object() {
        return false;
    }
    match value["type"].as_str() {
        Some("containerAuto") => {
            optional(value, "fileIds", strings)
                && optional(value, "memoryLimit", |v| {
                    choices(v, &["1g", "4g", "16g", "64g"])
                })
                && optional(value, "skills", skills)
                && optional(value, "networkPolicy", |policy| {
                    policy.is_object()
                        && match policy["type"].as_str() {
                            Some("disabled") => true,
                            Some("allowlist") => {
                                strings(&policy["allowedDomains"])
                                    && optional(policy, "domainSecrets", |secrets| {
                                        secrets.as_array().is_some_and(|secrets| {
                                            secrets.iter().all(|secret| {
                                                secret.is_object()
                                                    && secret["domain"].is_string()
                                                    && secret["name"].is_string()
                                                    && secret["value"].is_string()
                                            })
                                        })
                                    })
                            }
                            _ => false,
                        }
                })
        }
        Some("containerReference") => value["containerId"].is_string(),
        Some("local") | None
            if value
                .get("type")
                .is_none_or(|kind| matches!(kind.as_str(), Some("local"))) =>
        {
            optional(value, "skills", |skills| {
                skills.as_array().is_some_and(|skills| {
                    skills.iter().all(|skill| {
                        skill.is_object()
                            && skill["name"].is_string()
                            && skill["description"].is_string()
                            && skill["path"].is_string()
                    })
                })
            })
        }
        _ => false,
    }
}

pub(crate) fn validate_tool_args(kind: &str, args: &Value) -> Result<(), AiMuxError> {
    // These tools have no argument schema in the upstream preparation path.
    if matches!(
        kind,
        "local_shell" | "apply_patch" | "computer" | "computer_use" | "programmatic_tool_calling"
    ) {
        return Ok(());
    }
    let valid =
        args.is_object()
            && match kind {
                "web_search" | "web_search_preview" => {
                    optional(args, "searchContextSize", |v| {
                        choices(v, &["low", "medium", "high"])
                    }) && optional(args, "userLocation", location)
                        && (kind != "web_search"
                            || (optional(args, "externalWebAccess", Value::is_boolean)
                                && optional(args, "filters", |v| {
                                    v.is_object()
                                        && optional(v, "allowedDomains", strings)
                                        && optional(v, "blockedDomains", strings)
                                })))
                }
                "file_search" => {
                    strings(&args["vectorStoreIds"])
                        && optional(args, "maxNumResults", Value::is_number)
                        && optional(args, "filters", filter)
                        && optional(args, "ranking", |ranking| {
                            ranking.is_object()
                                && optional(ranking, "ranker", Value::is_string)
                                && optional(ranking, "scoreThreshold", Value::is_number)
                        })
                }
                "code_interpreter" => optional(args, "container", |container| {
                    container.is_string()
                        || (container.is_object() && optional(container, "fileIds", strings))
                }),
                "image_generation" => {
                    let allowed = [
                        "action",
                        "background",
                        "inputFidelity",
                        "inputImageMask",
                        "model",
                        "moderation",
                        "outputCompression",
                        "outputFormat",
                        "partialImages",
                        "quality",
                        "size",
                    ];
                    args.as_object().is_some_and(|object| {
                        object.keys().all(|key| allowed.contains(&key.as_str()))
                    }) && optional(args, "action", |v| {
                        choices(v, &["generate", "edit", "auto"])
                    }) && optional(args, "background", |v| {
                        choices(v, &["auto", "opaque", "transparent"])
                    }) && optional(args, "inputFidelity", |v| choices(v, &["low", "high"]))
                        && optional(args, "inputImageMask", |mask| {
                            mask.is_object() && string_fields(mask, &["fileId", "imageUrl"])
                        })
                        && optional(args, "model", Value::is_string)
                        && optional(args, "moderation", |v| choices(v, &["auto", "low"]))
                        && optional(args, "outputFormat", |v| {
                            choices(v, &["png", "jpeg", "webp"])
                        })
                        && optional(args, "quality", |v| {
                            choices(v, &["auto", "low", "medium", "high", "xhigh", "max"])
                        })
                        && optional(args, "outputCompression", |v| {
                            v.as_f64()
                                .is_some_and(|n| n.fract() == 0.0 && (0.0..=100.0).contains(&n))
                        })
                        && optional(args, "partialImages", |v| {
                            v.as_f64()
                                .is_some_and(|n| n.fract() == 0.0 && (0.0..=3.0).contains(&n))
                        })
                        && optional(args, "size", |v| {
                            v.as_str().is_some_and(|size| {
                                size == "auto"
                                    || size.split_once('x').is_some_and(|(w, h)| {
                                        !w.is_empty()
                                            && !h.is_empty()
                                            && w.bytes().all(|b| b.is_ascii_digit())
                                            && h.bytes().all(|b| b.is_ascii_digit())
                                    })
                            })
                        })
                }
                "mcp" => {
                    args["serverLabel"].is_string()
                        && string_fields(
                            args,
                            &[
                                "authorization",
                                "connectorId",
                                "serverDescription",
                                "serverUrl",
                            ],
                        )
                        && (args.get("serverUrl").is_some() || args.get("connectorId").is_some())
                        && optional(args, "headers", string_record)
                        && optional(args, "allowedTools", |tools| {
                            strings(tools)
                                || (tools.is_object()
                                    && optional(tools, "readOnly", Value::is_boolean)
                                    && optional(tools, "toolNames", strings))
                        })
                        && optional(args, "requireApproval", |approval| {
                            choices(approval, &["always", "never"])
                                || (approval.is_object()
                                    && optional(approval, "never", |never| {
                                        never.is_object() && optional(never, "toolNames", strings)
                                    }))
                        })
                }
                "shell" => optional(args, "environment", environment),
                "tool_search" => {
                    optional(args, "execution", |v| choices(v, &["server", "client"]))
                        && optional(args, "description", Value::is_string)
                        && optional(args, "parameters", Value::is_object)
                }
                "custom" => {
                    optional(args, "description", Value::is_string)
                        && optional(args, "async", Value::is_boolean)
                        && optional(args, "format", |format| {
                            format.is_object()
                                && match format["type"].as_str() {
                                    Some("text") => true,
                                    Some("grammar") => {
                                        choices(&format["syntax"], &["regex", "lark"])
                                            && format["definition"].is_string()
                                    }
                                    _ => false,
                                }
                        })
                }
                _ => true,
            };
    if valid {
        Ok(())
    } else {
        Err(AiMuxError::InvalidArgument(format!(
            "Invalid arguments for provider tool openai.{kind}"
        )))
    }
}

fn safety_checks(value: &Value) -> bool {
    value.as_array().is_some_and(|checks| {
        checks.iter().all(|check| {
            check.is_object()
                && check["id"].is_string()
                && string_fields(check, &["code", "message"])
        })
    })
}

fn computer_action(value: &Value) -> bool {
    if !value.is_object() {
        return false;
    }
    match value["type"].as_str() {
        Some("click" | "double_click" | "move" | "scroll") => {
            value["x"].is_number()
                && value["y"].is_number()
                && optional(value, "keys", strings)
                && (value["type"] != "click"
                    || choices(
                        &value["button"],
                        &["left", "right", "wheel", "back", "forward"],
                    ))
                && (value["type"] != "scroll"
                    || (value["scrollX"].is_number() && value["scrollY"].is_number()))
        }
        Some("drag") => {
            optional(value, "keys", strings)
                && value["path"].as_array().is_some_and(|points| {
                    points.iter().all(|point| {
                        point.is_object() && point["x"].is_number() && point["y"].is_number()
                    })
                })
        }
        Some("keypress") => strings(&value["keys"]),
        Some("type") => value["text"].is_string(),
        Some("screenshot" | "wait") => true,
        _ => false,
    }
}

pub(super) fn validate_tool_call(kind: &str, input: &Value) -> Result<(), AiMuxError> {
    let valid = if kind == "custom" {
        input.is_string()
    } else {
        input.is_object()
            && match kind {
                "local_shell" => {
                    let action = &input["action"];
                    action.is_object()
                        && action["type"] == "exec"
                        && strings(&action["command"])
                        && optional(action, "timeoutMs", Value::is_number)
                        && string_fields(action, &["user", "workingDirectory"])
                        && optional(action, "env", string_record)
                }
                "shell" => {
                    let action = &input["action"];
                    action.is_object()
                        && strings(&action["commands"])
                        && optional(action, "timeoutMs", Value::is_number)
                        && optional(action, "maxOutputLength", Value::is_number)
                }
                "apply_patch" => {
                    let operation = &input["operation"];
                    input["callId"].is_string()
                        && operation.is_object()
                        && operation["path"].is_string()
                        && match operation["type"].as_str() {
                            Some("create_file" | "update_file") => operation["diff"].is_string(),
                            Some("delete_file") => true,
                            _ => false,
                        }
                }
                "computer" => {
                    input["actions"]
                        .as_array()
                        .is_some_and(|actions| actions.iter().all(computer_action))
                        && safety_checks(&input["pendingSafetyChecks"])
                        && choices(
                            &input["status"],
                            &["in_progress", "completed", "incomplete"],
                        )
                }
                "programmatic_tool_calling" => {
                    input["code"].is_string() && input["fingerprint"].is_string()
                }
                "tool_search" => optional(input, "call_id", |id| id.is_null() || id.is_string()),
                _ => true,
            }
    };
    if valid {
        Ok(())
    } else {
        Err(AiMuxError::InvalidArgument(format!(
            "Invalid input for provider tool openai.{kind}"
        )))
    }
}

pub(super) fn validate_tool_result(kind: &str, output: &Value) -> Result<(), AiMuxError> {
    // Custom tools deliberately do not define an output schema upstream.
    if kind == "custom" {
        return Ok(());
    }
    let valid = output.is_object()
        && match kind {
            "local_shell" => output["output"].is_string(),
            "shell" => output["output"].as_array().is_some_and(|items| {
                items.iter().all(|item| {
                    let outcome = &item["outcome"];
                    item.is_object()
                        && item["stdout"].is_string()
                        && item["stderr"].is_string()
                        && outcome.is_object()
                        && match outcome["type"].as_str() {
                            Some("timeout") => true,
                            Some("exit") => outcome["exitCode"].is_number(),
                            _ => false,
                        }
                })
            }),
            "apply_patch" => {
                choices(&output["status"], &["completed", "failed"])
                    && optional(output, "output", Value::is_string)
            }
            "computer" => {
                let screenshot = &output["output"];
                screenshot.is_object()
                    && screenshot["type"] == "computer_screenshot"
                    && (screenshot["imageUrl"].is_string() || screenshot["fileId"].is_string())
                    && string_fields(screenshot, &["imageUrl", "fileId"])
                    && optional(screenshot, "detail", |detail| {
                        choices(detail, &["auto", "low", "high", "original"])
                    })
                    && optional(output, "acknowledgedSafetyChecks", safety_checks)
            }
            "programmatic_tool_calling" => {
                output["result"].is_string()
                    && choices(&output["status"], &["completed", "incomplete"])
            }
            "tool_search" => output["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().all(Value::is_object)),
            _ => true,
        };
    if valid {
        Ok(())
    } else {
        Err(AiMuxError::InvalidArgument(format!(
            "Invalid output for provider tool openai.{kind}"
        )))
    }
}
