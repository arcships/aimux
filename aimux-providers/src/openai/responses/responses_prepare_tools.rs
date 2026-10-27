//! Tool preparation for the Responses API (openai-responses-prepare-tools.ts).

use aimux_core::AiMuxError;
use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::Warning;
use serde_json::{Value, json};

use crate::openai::responses::tool_args::validate_tool_args;

/// The result of preparing tools for a Responses request body.
#[derive(Debug, Clone)]
pub struct PreparedResponsesTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub tool_warnings: Vec<Warning>,
}

/// Prepare the upstream Responses function and provider tool family.
///
/// # Errors
/// Returns an error for invalid provider tool arguments or conflicting namespaces.
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
) -> Result<PreparedResponsesTools, AiMuxError> {
    prepare_responses_tools_for(tools, tool_choice, None, true)
}

pub(crate) fn prepare_responses_tools_for(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
    allowed_tools: Option<&Value>,
    supports_async: bool,
) -> Result<PreparedResponsesTools, AiMuxError> {
    let Some(tools) = tools.as_ref().filter(|tools| !tools.is_empty()) else {
        return Ok(PreparedResponsesTools {
            tools: None,
            tool_choice: None,
            tool_warnings: Vec::new(),
        });
    };
    let mut tool_warnings = Vec::new();
    let mut prepared: Vec<Value> = Vec::new();
    for tool in tools {
        match tool {
            Tool::Function(ft) => {
                let mut value = function_tool_to_json(ft);
                value["parameters"] = crate::openai::normalize_json_schema::normalize_json_schema(
                    &ft.input_schema,
                    &mut tool_warnings,
                )?;
                let opts = ft
                    .provider_options
                    .as_ref()
                    .and_then(|options| options.get("openai"));
                if let Some(opts) = opts {
                    for (key, wire) in [
                        ("async", "async"),
                        ("deferLoading", "defer_loading"),
                        ("allowedCallers", "allowed_callers"),
                    ] {
                        if let Some(v) = opts.get(key) {
                            value[wire] = v.clone();
                        }
                    }
                    if let Some(schema) = opts.get("outputSchema") {
                        value["output_schema"] =
                            crate::openai::normalize_json_schema::normalize_json_schema(
                                schema,
                                &mut tool_warnings,
                            )?;
                    }
                    resolve_async_tool_option(
                        &mut value,
                        supports_async,
                        &ft.name,
                        &mut tool_warnings,
                    );
                    if let Some(namespace) = opts.get("namespace") {
                        let name = namespace
                            .get("name")
                            .and_then(Value::as_str)
                            .ok_or_else(|| AiMuxError::invalid_argument("tool namespace name"))?;
                        let description = namespace
                            .get("description")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                AiMuxError::invalid_argument("tool namespace description")
                            })?;
                        if let Some(group) = prepared
                            .iter_mut()
                            .find(|group| group["type"] == "namespace" && group["name"] == name)
                        {
                            if group["description"] != description {
                                return Err(AiMuxError::UnsupportedFunctionality(format!(
                                    "conflicting descriptions for OpenAI tool namespace \"{name}\""
                                )));
                            }
                            if let Some(items) = group["tools"].as_array_mut() {
                                items.push(value);
                            }
                        } else {
                            prepared.push(json!({"type":"namespace", "name":name, "description":description, "tools":[value]}));
                        }
                        continue;
                    }
                }
                prepared.push(value);
            }
            Tool::Provider(tool) => {
                if let Some(mut value) = provider_tool_to_json(tool)? {
                    resolve_async_tool_option(
                        &mut value,
                        supports_async,
                        &tool.name,
                        &mut tool_warnings,
                    );
                    prepared.push(value);
                }
            }
        }
    }
    let mut tool_choice = tool_choice.map(|choice| match choice {
        LanguageModelV4ToolChoice::Auto => json!("auto"),
        LanguageModelV4ToolChoice::None => json!("none"),
        LanguageModelV4ToolChoice::Required => json!("required"),
        LanguageModelV4ToolChoice::Tool { tool_name } => {
            let tool = tools.iter().find_map(|tool| match tool {
                Tool::Provider(tool) if tool.name == *tool_name => Some(tool),
                _ => None,
            });
            match tool.map(|tool| tool.id.as_str()) {
                Some("openai.custom") => json!({"type":"custom", "name":tool_name}),
                Some(id)
                    if [
                        "openai.code_interpreter",
                        "openai.file_search",
                        "openai.image_generation",
                        "openai.web_search_preview",
                        "openai.web_search",
                        "openai.mcp",
                        "openai.apply_patch",
                        "openai.computer",
                        "openai.programmatic_tool_calling",
                    ]
                    .contains(&id) =>
                {
                    json!({"type":id.trim_start_matches("openai.")})
                }
                _ => json!({"type":"function", "name":tool_name}),
            }
        }
    });
    if let Some(allowed) = allowed_tools {
        tool_choice = Some(prepare_allowed_tools(allowed, tools, &mut tool_warnings)?);
    }
    Ok(PreparedResponsesTools {
        tools: Some(prepared),
        tool_choice,
        tool_warnings,
    })
}

fn resolve_async_tool_option(
    value: &mut Value,
    supported: bool,
    name: &str,
    warnings: &mut Vec<Warning>,
) {
    if !supported && value.get("async") == Some(&json!(true)) {
        if let Some(object) = value.as_object_mut() {
            object.remove("async");
        }
        warnings.push(Warning::Unsupported {
            feature: format!("async tool calling for \"{name}\""),
            details: Some("Async tool calling is only supported by GPT-6 and later models.".into()),
        });
    }
}

fn prepare_allowed_tools(
    allowed: &Value,
    tools: &[Tool],
    warnings: &mut Vec<Warning>,
) -> Result<Value, AiMuxError> {
    let invalid = || AiMuxError::invalid_argument("allowedTools");
    let names = allowed
        .get("toolNames")
        .and_then(Value::as_array)
        .filter(|names| !names.is_empty())
        .ok_or_else(invalid)?;
    let mode = allowed
        .get("mode")
        .map(|mode| {
            mode.as_str()
                .filter(|mode| ["auto", "required"].contains(mode))
                .ok_or_else(invalid)
        })
        .transpose()?
        .unwrap_or("auto");
    let mut entries = Vec::new();
    let mut dropped = Vec::new();
    for name in names {
        let name = name.as_str().ok_or_else(invalid)?;
        let direct = tools.iter().find(|tool| match tool {
            Tool::Function(tool) => tool.name == name,
            Tool::Provider(tool) => tool.name == name,
        });
        let aliases: Vec<_> = tools.iter().filter(|tool| matches!(tool, Tool::Provider(tool) if tool.id.as_str().strip_prefix("openai.") == Some(name) && tool.name != name)).collect();
        let tool = if let Some(tool) = direct {
            if !aliases.is_empty() {
                warnings.push(Warning::Unsupported { feature:format!("allowedTools entry \"{name}\""), details:Some("this name is both a tool name and the provider tool name of another tool in this request; the tool with this name is allowed".into()) });
            }
            Some(tool)
        } else {
            if aliases.len() > 1
                && aliases.iter().any(|tool| match tool {
                    Tool::Provider(tool) => tool.id == "openai.custom" || tool.id == "openai.mcp",
                    _ => false,
                })
            {
                warnings.push(Warning::Unsupported { feature:format!("allowedTools entry \"{name}\""), details:Some("several tools in this request share this provider tool name; use the tool name from the tools for this request instead".into()) });
                dropped.push(name);
                continue;
            }
            aliases.first().copied()
        };
        let entry = match tool {
            Some(Tool::Function(tool)) => {
                let opts = tool
                    .provider_options
                    .as_ref()
                    .and_then(|options| options.get("openai"));
                let reason = if opts.is_some_and(|opts| opts.contains_key("namespace")) {
                    Some(
                        "tools inside an OpenAI tool namespace are not visible to tool_choice.allowed_tools",
                    )
                } else if opts.is_some_and(|opts| opts.get("deferLoading") == Some(&json!(true))) {
                    Some("deferred tools are not visible to tool_choice.allowed_tools")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    warnings.push(Warning::Unsupported {
                        feature: format!("allowedTools entry \"{name}\""),
                        details: Some(format!(
                            "{reason}; the tool is removed from the allowed tools"
                        )),
                    });
                    dropped.push(name);
                    continue;
                }
                json!({"type":"function", "name":tool.name})
            }
            Some(Tool::Provider(tool)) => {
                let Some(value) = provider_tool_to_json(tool)? else {
                    entries.push(json!({"type":"function", "name":name}));
                    continue;
                };
                match value["type"].as_str() {
                    Some("custom") => json!({"type":"custom", "name":tool.name}),
                    Some("mcp") => json!({"type":"mcp", "server_label":value["server_label"]}),
                    Some("tool_search") => {
                        warnings.push(Warning::Unsupported { feature:format!("allowedTools entry \"{name}\""), details:Some("OpenAI does not support tool_search tools in tool_choice.allowed_tools; the tool is removed from the allowed tools".into()) });
                        dropped.push(name);
                        continue;
                    }
                    Some(kind) => json!({"type":kind}),
                    None => continue,
                }
            }
            None => {
                warnings.push(Warning::Unsupported { feature:format!("allowedTools entry \"{name}\""), details:Some("the tool is not part of the tools for this request and is sent as a function tool".into()) });
                json!({"type":"function", "name":name})
            }
        };
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(AiMuxError::UnsupportedFunctionality(format!(
            "allowedTools with only tools that cannot be allow-listed ({})",
            dropped.join(", ")
        )));
    }
    Ok(json!({"type":"allowed_tools", "mode":mode, "tools":entries}))
}

// Only configured fields are sent: absent optional values are omitted, not null.
pub(crate) fn map_tool_fields(value: &Value, fields: &[(&str, &str)]) -> Value {
    let mut result = json!({});
    for (name, wire) in fields {
        if let Some(value) = value.get(*name) {
            result[*wire] = value.clone();
        }
    }
    result
}

fn provider_tool_to_json(
    tool: &aimux_core::tool::ProviderTool,
) -> Result<Option<Value>, AiMuxError> {
    let kind = tool.id.as_str().strip_prefix("openai.").unwrap_or("");
    if ![
        "file_search",
        "local_shell",
        "shell",
        "apply_patch",
        "computer",
        "web_search_preview",
        "web_search",
        "code_interpreter",
        "image_generation",
        "mcp",
        "custom",
        "programmatic_tool_calling",
        "tool_search",
    ]
    .contains(&kind)
    {
        return Ok(None);
    }
    if [
        "local_shell",
        "apply_patch",
        "computer",
        "programmatic_tool_calling",
    ]
    .contains(&kind)
    {
        return Ok(Some(json!({"type":kind})));
    }
    let args = &Value::Object(tool.args.clone());
    validate_tool_args(kind, args)?;
    let fields: &[(&str, &str)] = match kind {
        "file_search" => &[
            ("vectorStoreIds", "vector_store_ids"),
            ("maxNumResults", "max_num_results"),
            ("filters", "filters"),
        ],
        "web_search_preview" | "web_search" => &[
            ("searchContextSize", "search_context_size"),
            ("userLocation", "user_location"),
        ],
        "image_generation" => &[
            ("action", "action"),
            ("background", "background"),
            ("inputFidelity", "input_fidelity"),
            ("model", "model"),
            ("moderation", "moderation"),
            ("partialImages", "partial_images"),
            ("quality", "quality"),
            ("outputCompression", "output_compression"),
            ("outputFormat", "output_format"),
            ("size", "size"),
        ],
        "mcp" => &[
            ("serverLabel", "server_label"),
            ("authorization", "authorization"),
            ("connectorId", "connector_id"),
            ("headers", "headers"),
            ("serverDescription", "server_description"),
            ("serverUrl", "server_url"),
        ],
        "custom" => &[
            ("description", "description"),
            ("format", "format"),
            ("async", "async"),
        ],
        "tool_search" => &[
            ("execution", "execution"),
            ("description", "description"),
            ("parameters", "parameters"),
        ],
        _ => &[],
    };
    let mut value = map_tool_fields(args, fields);
    value["type"] = json!(kind);
    if let Some(location) = args.get("userLocation") {
        value["user_location"] = map_tool_fields(
            location,
            &[
                ("type", "type"),
                ("country", "country"),
                ("city", "city"),
                ("region", "region"),
                ("timezone", "timezone"),
            ],
        );
    }
    match kind {
        "file_search" => {
            if let Some(ranking) = args.get("ranking") {
                value["ranking_options"] = map_tool_fields(
                    ranking,
                    &[("ranker", "ranker"), ("scoreThreshold", "score_threshold")],
                );
            }
        }
        "web_search" => {
            if let Some(filters) = args.get("filters") {
                value["filters"] = map_tool_fields(
                    filters,
                    &[
                        ("allowedDomains", "allowed_domains"),
                        ("blockedDomains", "blocked_domains"),
                    ],
                );
            }
            if let Some(access) = args.get("externalWebAccess") {
                value["external_web_access"] = access.clone();
            }
        }
        "code_interpreter" => {
            value["container"] = match args.get("container") {
                Some(container) if container.is_string() => container.clone(),
                container => {
                    let mut value = container
                        .map(|container| map_tool_fields(container, &[("fileIds", "file_ids")]))
                        .unwrap_or_else(|| json!({}));
                    value["type"] = json!("auto");
                    value
                }
            }
        }
        "image_generation" => {
            if let Some(mask) = args.get("inputImageMask") {
                value["input_image_mask"] =
                    map_tool_fields(mask, &[("fileId", "file_id"), ("imageUrl", "image_url")]);
            }
        }
        "custom" => {
            value["name"] = json!(tool.name);
            if let Some(format) = args.get("format") {
                value["format"] = map_tool_fields(
                    format,
                    &[
                        ("type", "type"),
                        ("syntax", "syntax"),
                        ("definition", "definition"),
                    ],
                );
            }
        }
        "mcp" => {
            if let Some(allowed) = args.get("allowedTools") {
                value["allowed_tools"] = if allowed.is_array() {
                    allowed.clone()
                } else {
                    map_tool_fields(
                        allowed,
                        &[("readOnly", "read_only"), ("toolNames", "tool_names")],
                    )
                };
            }
            value["require_approval"] = args
                .get("requireApproval")
                .map(|approval| {
                    if approval.is_string() {
                        approval.clone()
                    } else if let Some(never) = approval.get("never") {
                        json!({"never":map_tool_fields(never, &[("toolNames", "tool_names")])})
                    } else {
                        json!("never")
                    }
                })
                .unwrap_or_else(|| json!("never"));
        }
        "shell" => {
            if let Some(environment) = args.get("environment") {
                value["environment"] = map_shell_environment(environment)?;
            }
        }
        _ => {}
    }
    Ok(Some(value))
}

fn map_shell_environment(environment: &Value) -> Result<Value, AiMuxError> {
    let kind = environment
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("local");
    let mut value = match kind {
        "containerReference" => map_tool_fields(environment, &[("containerId", "container_id")]),
        "containerAuto" => map_tool_fields(
            environment,
            &[("fileIds", "file_ids"), ("memoryLimit", "memory_limit")],
        ),
        _ => map_tool_fields(environment, &[("skills", "skills")]),
    };
    value["type"] = json!(match kind {
        "containerReference" => "container_reference",
        "containerAuto" => "container_auto",
        _ => "local",
    });
    if kind == "containerAuto" {
        if let Some(policy) = environment.get("networkPolicy") {
            value["network_policy"] = map_tool_fields(
                policy,
                &[
                    ("type", "type"),
                    ("allowedDomains", "allowed_domains"),
                    ("domainSecrets", "domain_secrets"),
                ],
            );
        }
        if let Some(skills) = environment.get("skills").and_then(Value::as_array) {
            value["skills"] = Value::Array(skills.iter().map(|skill| {
                if skill["type"] == "skillReference" {
                    let reference = skill.get("providerReference").and_then(Value::as_object).map(|reference| reference.iter().filter_map(|(provider, id)| Some((provider.clone(), id.as_str()?.to_owned()))).collect()).unwrap_or_default();
                    let id = crate::openai::chat::convert_to_chat_messages::resolve_provider_reference(&reference, "openai").map_err(AiMuxError::invalid_argument)?;
                    Ok(json!({"type":"skill_reference", "skill_id":id, "version":skill.get("version").cloned().unwrap_or_else(|| json!("latest"))}))
                } else {
                    let mut value = map_tool_fields(skill, &[("name", "name"), ("description", "description")]);
                    value["type"] = json!("inline");
                    value["source"] = map_tool_fields(&skill["source"], &[("type", "type"), ("mediaType", "media_type"), ("data", "data")]);
                    Ok(value)
                }
            }).collect::<Result<Vec<_>, AiMuxError>>()?);
        }
    }
    Ok(value)
}

/// Convert a `FunctionTool` into the Responses `function` tool JSON shape.
fn function_tool_to_json(t: &FunctionTool) -> Value {
    let mut func = json!({
        "type": "function",
        "name": t.name,
        "parameters": t.input_schema,
    });
    if let Some(ref desc) = t.description {
        func["description"] = json!(desc);
    }
    func["strict"] = json!(t.strict.unwrap_or(false));
    func
}
