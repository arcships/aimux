//! Tool preparation for the Anthropic provider.
//!
//! Faithful Rust port of the function-tool and tool-choice handling of the
//! TypeScript `prepareTools` in `packages/anthropic/src/anthropic-prepare-tools.ts`,
//! plus provider-defined tools (computer use, web search, code execution, ...)
//! via [`prepare_tools_with_provider`].
//!
//! Cache-control resolution is out of scope: it would require a
//! `CacheControlValidator` plus provider options on individual parts.

use std::collections::BTreeSet;

use aimux_core::tool::FunctionTool;
use aimux_core::tool::ToolChoice;
use aimux_core::types::Warning;
use serde_json::{Value, json};

/// Result of [`prepare_tools`] / [`prepare_tools_with_provider`].
#[derive(Debug, Clone)]
pub struct PreparedTools {
    /// Anthropic tool definitions, or `None` when there are no tools
    /// (or when tool choice `none` drops them).
    pub tools: Option<Vec<Value>>,
    /// Anthropic `tool_choice` value, or `None` when not applicable.
    pub tool_choice: Option<Value>,
    /// Warnings emitted while preparing tools.
    pub tool_warnings: Vec<Warning>,
    /// Beta headers required by the prepared tools.
    pub betas: BTreeSet<String>,
}

const BETA_STRUCTURED_OUTPUTS: &str = "structured-outputs-2025-11-13";
const BETA_ADVANCED_TOOL_USE: &str = "advanced-tool-use-2025-11-20";

/// Prepare `FunctionTool`s into the Anthropic `tools` / `tool_choice` shape.
///
/// `disable_parallel_tool_use` mirrors the TS `disableParallelToolUse` flag and
/// is attached to the chosen `tool_choice` when set. `default_eager_input_streaming`
/// is the model-level default for `eager_input_streaming` on function tools.
#[must_use]
pub fn prepare_tools(
    tools: Option<&[FunctionTool]>,
    tool_choice: Option<&ToolChoice>,
    disable_parallel_tool_use: bool,
    supports_structured_output: bool,
    supports_strict_tools: bool,
    default_eager_input_streaming: bool,
) -> PreparedTools {
    // Empty arrays are coerced to "no tools" to match the TS behaviour.
    let non_empty = tools.filter(|&t| !t.is_empty());

    let mut tool_warnings: Vec<Warning> = Vec::new();
    let mut betas: BTreeSet<String> = BTreeSet::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mut anthropic_tools: Vec<Value> = Vec::new();
            for tool in tools {
                anthropic_tools.push(prepare_function_tool(
                    tool,
                    supports_structured_output,
                    supports_strict_tools,
                    default_eager_input_streaming,
                    &mut betas,
                    &mut tool_warnings,
                ));
            }
            Some(anthropic_tools)
        }
    };

    // If the tools were dropped (e.g. there never were any), there is no
    // tool_choice to emit either.
    let tools_opt = match tools_opt {
        None => {
            return PreparedTools {
                tools: None,
                tool_choice: None,
                tool_warnings,
                betas,
            };
        }
        Some(t) => Some(t),
    };

    // Anthropic does not support 'none' tool choice, so the tools are removed.
    if matches!(tool_choice, Some(ToolChoice::None)) {
        return PreparedTools {
            tools: None,
            tool_choice: None,
            tool_warnings,
            betas,
        };
    }

    let tool_choice_opt = build_tool_choice(tool_choice, disable_parallel_tool_use);

    PreparedTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
        betas,
    }
}

// =============================================================================
// Provider-defined tools (computer use, web search, code execution, ...)
// =============================================================================

/// A tool definition that may be either a function tool or a provider-defined
/// tool (e.g. `anthropic.computer_20241022`). Mirrors the Vercel AI SDK
/// `LanguageModelV4CallOptions['tools']` discriminated union.
#[derive(Debug, Clone)]
pub enum AnthropicTool {
    /// A user-defined function tool.
    Function(FunctionTool),
    /// A provider-defined tool, identified by its `id` (e.g.
    /// `anthropic.computer_20241022`), with a display `name` and tool-specific
    /// `args` (a JSON object with camelCase keys).
    Provider {
        id: String,
        name: String,
        args: Value,
    },
}

/// Read a camelCase arg from the `args` object, returning `Value::Null` when
/// absent (mirroring the TS `undefined`-when-absent field behaviour).
fn arg(args: &Value, key: &str) -> Value {
    args.get(key).cloned().unwrap_or(Value::Null)
}

/// Prepare a mix of function and provider-defined tools into the Anthropic
/// `tools` / `tool_choice` shape. This is the provider-tool-aware counterpart of
/// [`prepare_tools`]; it mirrors the TS `prepareTools` `case 'provider'` branch.
#[must_use]
pub fn prepare_tools_with_provider(
    tools: Option<&[AnthropicTool]>,
    tool_choice: Option<&ToolChoice>,
    disable_parallel_tool_use: bool,
    supports_structured_output: bool,
    supports_strict_tools: bool,
    default_eager_input_streaming: bool,
) -> PreparedTools {
    // Empty arrays are coerced to "no tools" to match the TS behaviour.
    let non_empty = tools.filter(|&t| !t.is_empty());

    let mut tool_warnings: Vec<Warning> = Vec::new();
    let mut betas: BTreeSet<String> = BTreeSet::new();

    let tools_opt = match non_empty {
        None => None,
        Some(tools) => {
            let mut anthropic_tools: Vec<Value> = Vec::new();
            for tool in tools {
                match tool {
                    AnthropicTool::Function(ft) => {
                        anthropic_tools.push(prepare_function_tool(
                            ft,
                            supports_structured_output,
                            supports_strict_tools,
                            default_eager_input_streaming,
                            &mut betas,
                            &mut tool_warnings,
                        ));
                    }
                    AnthropicTool::Provider { id, name: _, args } => {
                        if let Some(def) = prepare_provider_tool(id, args, &mut betas) {
                            anthropic_tools.push(def);
                        } else {
                            tool_warnings.push(Warning::Unsupported {
                                feature: format!("provider-defined tool {id}"),
                                details: None,
                            });
                        }
                    }
                }
            }
            Some(anthropic_tools)
        }
    };

    let tools_opt = match tools_opt {
        None => {
            return PreparedTools {
                tools: None,
                tool_choice: None,
                tool_warnings,
                betas,
            };
        }
        Some(t) => Some(t),
    };

    // Anthropic does not support 'none' tool choice, so the tools are removed.
    if matches!(tool_choice, Some(ToolChoice::None)) {
        return PreparedTools {
            tools: None,
            tool_choice: None,
            tool_warnings,
            betas,
        };
    }

    let tool_choice_opt = build_tool_choice(tool_choice, disable_parallel_tool_use);

    PreparedTools {
        tools: tools_opt,
        tool_choice: tool_choice_opt,
        tool_warnings,
        betas,
    }
}

/// Build the Anthropic `tool_choice` value from the unified `ToolChoice`.
fn build_tool_choice(tool_choice: Option<&ToolChoice>, disable_parallel: bool) -> Option<Value> {
    let with_disable = |v: Value| -> Value {
        let mut v = v;
        if disable_parallel {
            v["disable_parallel_tool_use"] = json!(true);
        }
        v
    };
    match tool_choice {
        None => {
            if disable_parallel {
                Some(json!({ "type": "auto", "disable_parallel_tool_use": true }))
            } else {
                None
            }
        }
        Some(ToolChoice::Auto) => Some(with_disable(json!({ "type": "auto" }))),
        Some(ToolChoice::Required) => Some(with_disable(json!({ "type": "any" }))),
        // Anthropic does not support 'none' tool choice, so the tools are removed.
        // (The caller is responsible for dropping `tools` when this is returned;
        // here we only signal the choice is absent.)
        Some(ToolChoice::None) => None,
        Some(ToolChoice::Tool { tool_name }) => {
            Some(with_disable(json!({ "type": "tool", "name": tool_name })))
        }
    }
}

/// Prepare a single function tool into an Anthropic tool definition. Shared
/// between [`prepare_tools`] and [`prepare_tools_with_provider`].
fn prepare_function_tool(
    tool: &FunctionTool,
    supports_structured_output: bool,
    supports_strict_tools: bool,
    default_eager_input_streaming: bool,
    betas: &mut BTreeSet<String>,
    tool_warnings: &mut Vec<Warning>,
) -> Value {
    let anthropic_options = tool
        .provider_options
        .as_ref()
        .and_then(|po| po.get("anthropic"));

    let eager_input_streaming = anthropic_options
        .and_then(|o| o.get("eagerInputStreaming"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(default_eager_input_streaming);
    let defer_loading = anthropic_options
        .and_then(|o| o.get("deferLoading"))
        .and_then(serde_json::Value::as_bool);
    let allowed_callers = anthropic_options.and_then(|o| o.get("allowedCallers"));

    #[allow(clippy::collapsible_if, reason = "let-chain not stable on 1.97")]
    if !supports_strict_tools {
        if let Some(strict) = tool.strict {
            tool_warnings.push(Warning::Unsupported {
                feature: "strict".to_string(),
                details: Some(format!(
                    "Tool '{}' has strict: {}, but strict mode is not supported by this provider. The strict property will be ignored.",
                    tool.name, strict
                )),
            });
        }
    }

    let mut def = json!({
        "name": tool.name,
        "input_schema": tool.input_schema,
    });
    if let Some(ref desc) = tool.description {
        def["description"] = json!(desc);
    }
    if eager_input_streaming {
        def["eager_input_streaming"] = json!(true);
    }
    #[allow(clippy::collapsible_if, reason = "let-chain not stable on 1.97")]
    if supports_strict_tools {
        if let Some(strict) = tool.strict {
            def["strict"] = json!(strict);
        }
    }
    if let Some(dl) = defer_loading {
        def["defer_loading"] = json!(dl);
    }
    if let Some(ac) = allowed_callers {
        def["allowed_callers"] = ac.clone();
    }
    if let Some(ref examples) = tool.input_examples {
        let inputs: Vec<Value> = examples
            .iter()
            .map(|e| e.get("input").cloned().unwrap_or(e.clone()))
            .collect();
        def["input_examples"] = json!(inputs);
    }

    if supports_structured_output {
        betas.insert(BETA_STRUCTURED_OUTPUTS.to_string());
    }
    if tool.input_examples.is_some() || allowed_callers.is_some() {
        betas.insert(BETA_ADVANCED_TOOL_USE.to_string());
    }

    def
}

/// Map a provider-defined tool `id` to its Anthropic tool definition, returning
/// `None` (so the caller emits an "unsupported" warning) when the id is unknown.
pub(crate) fn prepare_provider_tool(
    id: &str,
    args: &Value,
    betas: &mut BTreeSet<String>,
) -> Option<Value> {
    Some(match id {
        "anthropic.computer_20241022" => {
            betas.insert("computer-use-2024-10-22".to_string());
            json!({
                "name": "computer",
                "type": "computer_20241022",
                "display_width_px": arg(args, "displayWidthPx"),
                "display_height_px": arg(args, "displayHeightPx"),
                "display_number": arg(args, "displayNumber"),
            })
        }
        "anthropic.computer_20250124" => {
            betas.insert("computer-use-2025-01-24".to_string());
            json!({
                "name": "computer",
                "type": "computer_20250124",
                "display_width_px": arg(args, "displayWidthPx"),
                "display_height_px": arg(args, "displayHeightPx"),
                "display_number": arg(args, "displayNumber"),
            })
        }
        "anthropic.computer_20251124" => {
            betas.insert("computer-use-2025-11-24".to_string());
            json!({
                "name": "computer",
                "type": "computer_20251124",
                "display_width_px": arg(args, "displayWidthPx"),
                "display_height_px": arg(args, "displayHeightPx"),
                "display_number": arg(args, "displayNumber"),
                "enable_zoom": arg(args, "enableZoom"),
            })
        }
        "anthropic.text_editor_20241022" => {
            betas.insert("computer-use-2024-10-22".to_string());
            json!({ "name": "str_replace_editor", "type": "text_editor_20241022" })
        }
        "anthropic.text_editor_20250124" => {
            betas.insert("computer-use-2025-01-24".to_string());
            json!({ "name": "str_replace_editor", "type": "text_editor_20250124" })
        }
        "anthropic.text_editor_20250429" => {
            betas.insert("computer-use-2025-01-24".to_string());
            json!({ "name": "str_replace_based_edit_tool", "type": "text_editor_20250429" })
        }
        "anthropic.text_editor_20250728" => {
            json!({
                "name": "str_replace_based_edit_tool",
                "type": "text_editor_20250728",
                "max_characters": arg(args, "maxCharacters"),
            })
        }
        "anthropic.bash_20241022" => {
            betas.insert("computer-use-2024-10-22".to_string());
            json!({ "name": "bash", "type": "bash_20241022" })
        }
        "anthropic.bash_20250124" => {
            betas.insert("computer-use-2025-01-24".to_string());
            json!({ "name": "bash", "type": "bash_20250124" })
        }
        "anthropic.memory_20250818" => {
            betas.insert("context-management-2025-06-27".to_string());
            json!({ "name": "memory", "type": "memory_20250818" })
        }
        "anthropic.code_execution_20250522" => {
            betas.insert("code-execution-2025-05-22".to_string());
            json!({ "type": "code_execution_20250522", "name": "code_execution" })
        }
        "anthropic.code_execution_20250825" => {
            betas.insert("code-execution-2025-08-25".to_string());
            json!({ "type": "code_execution_20250825", "name": "code_execution" })
        }
        "anthropic.code_execution_20260120" => {
            json!({ "type": "code_execution_20260120", "name": "code_execution" })
        }
        "anthropic.web_search_20250305" => {
            json!({
                "type": "web_search_20250305",
                "name": "web_search",
                "max_uses": arg(args, "maxUses"),
                "allowed_domains": arg(args, "allowedDomains"),
                "blocked_domains": arg(args, "blockedDomains"),
                "user_location": arg(args, "userLocation"),
            })
        }
        "anthropic.web_search_20260209" => {
            betas.insert("code-execution-web-tools-2026-02-09".to_string());
            json!({
                "type": "web_search_20260209",
                "name": "web_search",
                "max_uses": arg(args, "maxUses"),
                "allowed_domains": arg(args, "allowedDomains"),
                "blocked_domains": arg(args, "blockedDomains"),
                "user_location": arg(args, "userLocation"),
            })
        }
        "anthropic.web_fetch_20250910" => {
            betas.insert("web-fetch-2025-09-10".to_string());
            json!({
                "type": "web_fetch_20250910",
                "name": "web_fetch",
                "max_uses": arg(args, "maxUses"),
                "allowed_domains": arg(args, "allowedDomains"),
                "blocked_domains": arg(args, "blockedDomains"),
                "citations": arg(args, "citations"),
                "max_content_tokens": arg(args, "maxContentTokens"),
            })
        }
        "anthropic.web_fetch_20260209" => {
            betas.insert("code-execution-web-tools-2026-02-09".to_string());
            json!({
                "type": "web_fetch_20260209",
                "name": "web_fetch",
                "max_uses": arg(args, "maxUses"),
                "allowed_domains": arg(args, "allowedDomains"),
                "blocked_domains": arg(args, "blockedDomains"),
                "citations": arg(args, "citations"),
                "max_content_tokens": arg(args, "maxContentTokens"),
            })
        }
        "anthropic.tool_search_regex_20251119" => {
            json!({ "name": "tool_search_tool_regex", "type": "tool_search_tool_regex_20251119" })
        }
        "anthropic.tool_search_bm25_20251119" => {
            json!({ "name": "tool_search_tool_bm25", "type": "tool_search_tool_bm25_20251119" })
        }
        "anthropic.advisor_20260301" => {
            betas.insert("advisor-tool-2026-03-01".to_string());
            let mut def = json!({
                "type": "advisor_20260301",
                "name": "advisor",
                "model": arg(args, "model"),
            });
            if args.get("maxUses").is_some() {
                def["max_uses"] = arg(args, "maxUses");
            }
            if args.get("caching").is_some() {
                def["caching"] = arg(args, "caching");
            }
            def
        }
        _ => return None,
    })
}
