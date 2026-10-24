//! `google-prepare-tools.ts`: function tools and provider-defined tools in
//! Gemini's `tools` + `toolConfig` shape.

use super::model_capabilities::{GoogleModelCapabilities, get_google_model_capabilities};
use aimux_core::options::LanguageModelV4ToolChoice;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::Warning;
use serde_json::{Map, Value, json};

/// Result of preparing tools for the Google request body.
#[derive(Debug, Clone, Default)]
pub struct PreparedTools {
    /// `tools` array (e.g. `[{ functionDeclarations: [...] }]`), or `None`
    /// when there are no usable tools.
    pub tools: Option<Vec<Value>>,
    /// `toolConfig` object (e.g. `{ functionCallingConfig: { mode: "AUTO" } }`).
    pub tool_config: Option<Value>,
}

/// Prepare `FunctionTool`s into the Google `tools` / `toolConfig` JSON shape.
///
/// Mirrors the function-tools path of the upstream `prepareTools`.
/// Use [`prepare_all_tools`] for provider-defined tools.
#[must_use]
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
) -> PreparedTools {
    // Coerce empty arrays to None (matches TS `tools?.length ? tools : undefined`).
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());

    let Some(tools) = non_empty else {
        return PreparedTools::default();
    };

    let mut has_strict = false;
    let function_declarations: Vec<Value> = tools
        .iter()
        .map(|t| {
            if t.strict == Some(true) {
                has_strict = true;
            }
            build_function_declaration(t)
        })
        .collect();

    let tools_value = Some(vec![
        json!({ "functionDeclarations": function_declarations }),
    ]);

    let tool_config = match tool_choice {
        None => has_strict.then(|| json!({ "functionCallingConfig": { "mode": "VALIDATED" } })),
        Some(LanguageModelV4ToolChoice::Auto) => {
            if has_strict {
                Some(json!({ "functionCallingConfig": { "mode": "VALIDATED" } }))
            } else {
                Some(json!({ "functionCallingConfig": { "mode": "AUTO" } }))
            }
        }
        Some(LanguageModelV4ToolChoice::None) => {
            Some(json!({ "functionCallingConfig": { "mode": "NONE" } }))
        }
        Some(LanguageModelV4ToolChoice::Required) => {
            Some(json!({ "functionCallingConfig": { "mode": "ANY" } }))
        }
        Some(LanguageModelV4ToolChoice::Tool { tool_name }) => Some(json!({
            "functionCallingConfig": { "mode": "ANY", "allowedFunctionNames": [tool_name] }
        })),
    };

    PreparedTools {
        tools: tools_value,
        tool_config,
    }
}

/// Result of preparing tools for the Google request body, including any
/// warnings about unsupported tools.
#[derive(Debug, Clone, Default)]
pub struct PreparedToolsWithWarnings {
    /// `tools` array, or `None` when there are no usable tools.
    pub tools: Option<Vec<Value>>,
    /// `toolConfig` object.
    pub tool_config: Option<Value>,
    /// Warnings about unsupported tools / combinations.
    pub warnings: Vec<Warning>,
}

/// Prepare function **and** provider-defined tools into the Google `tools` /
/// `toolConfig` JSON shape, mirroring the TS `prepareTools`.
///
/// Unlike [`prepare_tools`] (which only handles `FunctionTool`s), this handles
/// `Tool::Provider` entries (`google.google_search`, `google.code_execution`,
/// `google.url_context`, `google.google_maps`, `google.enterprise_web_search`,
/// `google.file_search`) and the Gemini 3 combined function+provider shape.
#[must_use]
pub fn prepare_all_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: Option<&LanguageModelV4ToolChoice>,
    model_id: &str,
) -> PreparedToolsWithWarnings {
    let caps = get_google_model_capabilities(model_id);
    let mut warnings: Vec<Warning> = Vec::new();

    // Coerce empty arrays to None (matches TS `tools?.length ? tools : undefined`).
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());
    let Some(tools) = non_empty else {
        return PreparedToolsWithWarnings::default();
    };

    let has_function_tools = tools.iter().any(|t| matches!(t, Tool::Function(_)));
    let has_provider_tools = tools.iter().any(|t| matches!(t, Tool::Provider(_)));

    if has_function_tools && has_provider_tools && !caps.uses_gemini_3_features {
        warnings.push(Warning::Unsupported {
            feature: "combination of function and provider-defined tools".to_string(),
            details: None,
        });
    }

    if has_provider_tools {
        let mut google_tools: Vec<Value> = Vec::new();

        for tool in tools.iter().filter_map(|t| match t {
            Tool::Provider(pt) => Some(pt),
            _ => None,
        }) {
            push_provider_tool(tool, &caps, &mut google_tools, &mut warnings);
        }

        // Gemini 3: keep function declarations alongside provider tools.
        if has_function_tools && caps.uses_gemini_3_features && !google_tools.is_empty() {
            let function_declarations: Vec<Value> = tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => Some(build_function_declaration(ft)),
                    _ => None,
                })
                .collect();

            let mut combined_config = Map::new();
            let fc = match tool_choice {
                Some(LanguageModelV4ToolChoice::None) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("NONE"));
                    m
                }
                Some(LanguageModelV4ToolChoice::Required) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("ANY"));
                    m
                }
                Some(LanguageModelV4ToolChoice::Tool { tool_name }) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("ANY"));
                    m.insert("allowedFunctionNames".to_string(), json!([tool_name]));
                    m
                }
                None | Some(LanguageModelV4ToolChoice::Auto) => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("VALIDATED"));
                    m
                }
            };
            combined_config.insert("functionCallingConfig".to_string(), Value::Object(fc));
            combined_config.insert("includeServerSideToolInvocations".to_string(), json!(true));

            let mut tools_value = google_tools;
            tools_value.push(json!({ "functionDeclarations": function_declarations }));

            return PreparedToolsWithWarnings {
                tools: Some(tools_value),
                tool_config: Some(Value::Object(combined_config)),
                warnings,
            };
        }

        let tools_value = if google_tools.is_empty() {
            None
        } else {
            Some(google_tools)
        };
        return PreparedToolsWithWarnings {
            tools: tools_value,
            tool_config: None,
            warnings,
        };
    }

    // Function-only path: delegate to the existing `prepare_tools`.
    let function_tools: Vec<FunctionTool> = tools
        .iter()
        .filter_map(|t| match t {
            Tool::Function(ft) => Some(ft.clone()),
            _ => None,
        })
        .collect();
    let prepared = prepare_tools(&Some(function_tools), tool_choice);
    PreparedToolsWithWarnings {
        tools: prepared.tools,
        tool_config: prepared.tool_config,
        warnings,
    }
}

/// Map a single provider-defined tool to its Google request-body entry, pushing
/// either the tool object or an `Unsupported` warning.
fn push_provider_tool(
    tool: &aimux_core::tool::ProviderTool,
    caps: &GoogleModelCapabilities,
    google_tools: &mut Vec<Value>,
    warnings: &mut Vec<Warning>,
) {
    let unsupported = |details: Option<&str>| Warning::Unsupported {
        feature: format!("provider-defined tool {}", tool.id),
        details: details.map(std::string::ToString::to_string),
    };
    match tool.id.as_str() {
        "google.google_search" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "googleSearch": tool.args }));
            } else {
                warnings.push(unsupported(Some(
                    "Google Search requires Gemini 2.0 or newer.",
                )));
            }
        }
        "google.enterprise_web_search" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "enterpriseWebSearch": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "Enterprise Web Search requires Gemini 2.0 or newer.",
                )));
            }
        }
        "google.url_context" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "urlContext": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The URL context tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.code_execution" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "codeExecution": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The code execution tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.file_search" => {
            if caps.supports_file_search {
                google_tools.push(json!({ "fileSearch": tool.args }));
            } else {
                warnings.push(unsupported(Some(
                    "The file search tool is only supported with Gemini 2.5 models and Gemini 3 models.",
                )));
            }
        }
        "google.vertex_rag_store" => {
            if caps.supports_gemini_2_tools {
                let mut rag =
                    json!({ "rag_resources": { "rag_corpus": tool.args.get("ragCorpus") } });
                if let Some(top_k) = tool.args.get("topK") {
                    rag["similarity_top_k"] = top_k.clone();
                }
                google_tools.push(json!({ "retrieval": { "vertex_rag_store": rag } }));
            } else {
                warnings.push(unsupported(Some(
                    "The RAG store tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.google_maps" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "googleMaps": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The Google Maps grounding tool is not supported with Gemini models other than Gemini 2 or newer.",
                )));
            }
        }
        _ => {
            warnings.push(unsupported(None));
        }
    }
}

/// Build a single `functionDeclarations` entry from a `FunctionTool`.
///
/// The tool's input schema goes out as `parametersJsonSchema`, unchanged, the
/// way `@ai-sdk/google` sends it (Gemini accepts JSON Schema natively). An
/// empty root object schema is preserved.
fn build_function_declaration(ft: &FunctionTool) -> Value {
    let mut decl = Map::new();
    decl.insert("name".to_string(), json!(ft.name));
    decl.insert(
        "description".to_string(),
        json!(ft.description.as_deref().unwrap_or("")),
    );
    decl.insert("parametersJsonSchema".to_string(), ft.input_schema.clone());
    Value::Object(decl)
}
