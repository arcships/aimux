//! What differs on Groq's chat endpoint, expressed as data and hooks of the
//! OpenAI-compatible chat model (the AI SDK's `GroqChatLanguageModel`).

use std::sync::Arc;

use serde_json::{Map, Value, json};

use aimux_core::content::ContentPart;
use aimux_core::options::CallOptions;
use aimux_core::tool::Tool;
use aimux_core::types::Warning;

use crate::openai_compatible::ChatProfile;
use crate::openai_compatible::config::{ChatDialect, ChatHooks, default_error_structure};
use crate::openai_compatible::convert::{PreparedTools, function_tool_value, tool_choice_value};

/// The provider tool id of Groq's browser search.
const BROWSER_SEARCH_TOOL: &str = "groq.browser_search";

/// Models that support Groq's `browser_search` tool.
const BROWSER_SEARCH_MODELS: [&str; 2] = ["openai/gpt-oss-20b", "openai/gpt-oss-120b"];

/// Groq's chat behavior:
///
/// - usage of a streaming response rides in `x_groq.usage`, and no
///   `stream_options` are requested;
/// - `top_k` is not supported;
/// - the token limit is `max_completion_tokens` (`max_tokens` is deprecated);
/// - structured outputs are on unless the call turns them off
///   (`structuredOutputs`), with `strictJsonSchema` defaulting to true;
/// - assistant turns carry `reasoning` and an empty-string `content`;
/// - the `groq.browser_search` provider tool maps to `{ "type": "browser_search" }`;
/// - the options `reasoningFormat`, `serviceTier` and `parallelToolCalls`
///   become `reasoning_format`, `service_tier` and `parallel_tool_calls`.
pub(crate) fn profile() -> ChatProfile {
    ChatProfile {
        include_usage: false,
        supports_structured_outputs: true,
        supports_multi_part_tool_content: false,
        dialect: ChatDialect {
            supports_top_k: false,
            supports_tools: true,
            supports_response_format: true,
            max_tokens_key: Some("max_completion_tokens"),
            stream_usage_key: Some("x_groq".to_string()),
            convert_usage: None,
            metadata_extractor: None,
            error_structure: Arc::new(default_error_structure),
            hooks: Arc::new(GroqHooks),
        },
    }
}

struct GroqHooks;

impl ChatHooks for GroqHooks {
    /// Mirrors `convertToGroqChatMessages`: text, reasoning and tool calls are
    /// collected together; `content` is `""` (not null) without text and
    /// `reasoning` appears only when non-empty.
    fn assistant_message(&self, content: &[ContentPart]) -> Option<Value> {
        let text: String = content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let reasoning: String = content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Reasoning { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let tool_calls: Vec<Value> = content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolCall {
                    tool_call_id,
                    tool_name,
                    input,
                    ..
                } => Some(json!({
                    "type": "function",
                    "id": tool_call_id,
                    "function": {
                        "name": tool_name,
                        "arguments": if input.is_null() { "{}".to_string() } else { input.to_string() },
                    }
                })),
                _ => None,
            })
            .collect();

        let mut message = Map::new();
        message.insert("role".into(), json!("assistant"));
        message.insert("content".into(), json!(text));
        if !reasoning.is_empty() {
            message.insert("reasoning".into(), json!(reasoning));
        }
        if !tool_calls.is_empty() {
            message.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        Some(Value::Object(message))
    }

    /// Function tools plus the `groq.browser_search` provider tool (only on
    /// the models that support it); other provider tools are warned about.
    fn prepare_tools(&self, options: &CallOptions, model_id: &str) -> PreparedTools {
        let mut prepared = PreparedTools::default();
        let Some(tools) = options.tools.as_ref().filter(|tools| !tools.is_empty()) else {
            return prepared;
        };
        let mut wire = Vec::new();
        for tool in tools {
            if let Tool::Function(function) = tool {
                wire.push(function_tool_value(function));
            }
        }
        for tool in tools {
            let Tool::Provider(provider) = tool else {
                continue;
            };
            if provider.id == BROWSER_SEARCH_TOOL {
                if BROWSER_SEARCH_MODELS.contains(&model_id) {
                    wire.push(json!({ "type": "browser_search" }));
                } else {
                    prepared.warnings.push(Warning::Unsupported {
                        feature: format!("provider-defined tool {}", provider.id),
                        details: Some(format!(
                            "Browser search is only supported on the following models: {}. Current model: {model_id}",
                            BROWSER_SEARCH_MODELS.join(", ")
                        )),
                    });
                }
            } else {
                prepared.warnings.push(Warning::Unsupported {
                    feature: format!("provider-defined tool {}", provider.id),
                    details: None,
                });
            }
        }
        if !wire.is_empty() {
            prepared.tools = Some(wire);
            prepared.tool_choice = Some(tool_choice_value(&options.tool_choice));
        }
        prepared
    }

    /// Groq takes images only: no provider file references, no other media.
    fn unsupported_file_part(&self, media_type: &str, reference: bool) -> Option<&'static str> {
        if reference {
            Some("file parts with provider references")
        } else if media_type.split('/').next() != Some("image") {
            Some("non-image file content parts")
        } else {
            None
        }
    }

    fn structured_outputs(&self, merged: &Map<String, Value>, configured: bool) -> bool {
        merged
            .get("structuredOutputs")
            .and_then(Value::as_bool)
            .unwrap_or(configured)
    }

    fn consumed_option_keys(&self) -> &'static [&'static str] {
        &[
            "structuredOutputs",
            "reasoningFormat",
            "serviceTier",
            "parallelToolCalls",
        ]
    }

    fn extend_body(
        &self,
        body: &mut Map<String, Value>,
        merged: &Map<String, Value>,
        _warnings: &mut Vec<Warning>,
    ) {
        for (option, field) in [
            ("parallelToolCalls", "parallel_tool_calls"),
            ("reasoningFormat", "reasoning_format"),
            ("serviceTier", "service_tier"),
        ] {
            if let Some(value) = merged.get(option) {
                body.insert(field.into(), value.clone());
            }
        }
    }
}
