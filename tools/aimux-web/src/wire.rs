//! Wire schema: the frontend-friendly message/options format (RFC-0029 §5.3)
//! and its conversion into aimux user-facing types (`ModelMessage` /
//! `GenerateTextOptions`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::generate::GenerateTextOptions;
use aimux_core::message::{MessageContent, ModelMessage, ModelPrompt, Role};
use aimux_core::tool::{FunctionTool, Tool};

// ─────────────────────────────────────────────────────────────────────────────
// Wire request types — the console's API surface.
//
// The corresponding TypeScript declarations live in `web/src/types/Wire*.ts`
// (committed). They are NOT ts-rs `#[ts(export)]`ed here: the repo's global
// `TS_RS_EXPORT_DIR` points at `bindings/node/src/types`, and these types
// belong to the web frontend, not the node bindings. Keep the committed
// `web/src/types/Wire*.ts` in sync when changing these structs.
// ─────────────────────────────────────────────────────────────────────────────

/// One model call from the console (RFC-0029 §5.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireCallRequest {
    /// Registry name or native protocol name ("openai", "deepseek", …).
    pub provider: String,
    /// Model id (free text, e.g. "gpt-4o").
    pub model: String,
    /// API key: `None` (provider reads its registered env var), or
    /// `Some("env:VAR")`. Plaintext keys are rejected (RFC-0029 §9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (proxies / local endpoints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Stream the response as SSE (`StreamPart` events). Default true.
    #[serde(default = "default_true")]
    pub stream: bool,
    /// Route through the loaded mock model (offline mode, RFC-0023 P3).
    #[serde(default)]
    pub mock: bool,
    #[serde(default)]
    pub options: WireOptions,
    /// Session grouping id (RFC-0024). The console reuses one id per agent run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Informational step marker (the backend assigns the authoritative step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    pub messages: Vec<WireMessage>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WireOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<WireTool>>,
    /// `"text"` or `{"json": …}` — passed through as `ResponseFormat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_overrides: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_raw_chunks: Option<bool>,
}

/// A function tool definition (JSON Schema parameters).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the parameters.
    pub parameters: Value,
}

/// A chat message in the wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    /// "system" | "user" | "assistant" | "tool".
    pub role: String,
    pub content: Vec<WireContentPart>,
}

/// A content part in the wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireContentPart {
    Text {
        text: String,
    },
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        input: Value,
    },
    ToolResult {
        tool_call_id: String,
        result: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Response for non-streaming calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireCallResponse {
    pub text: String,
    pub finish_reason: Value,
    pub usage: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<WireMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// SSE `meta` event payload — the frontend trace anchor (RFC-0029 §5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMeta {
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    pub outcome: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Conversions
// ─────────────────────────────────────────────────────────────────────────────

/// Wire content part → `ContentPart`.
fn to_content_part(p: &WireContentPart) -> ContentPart {
    match p {
        WireContentPart::Text { text } => ContentPart::Text {
            text: text.clone(),
            provider_options: None,
        },
        WireContentPart::ToolCall {
            tool_call_id,
            tool_name,
            input,
        } => ContentPart::ToolCall {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            input: input.clone(),
            thought_signature: None,
            provider_options: None,
        },
        WireContentPart::ToolResult {
            tool_call_id,
            result,
            is_error,
        } => ContentPart::ToolResult {
            tool_call_id: tool_call_id.clone(),
            result: result.clone(),
            tool_name: None,
            is_error: *is_error,
            preliminary: None,
            dynamic: None,
            provider_options: None,
        },
    }
}

/// Wire messages → user-facing `ModelPrompt`.
pub fn to_model_prompt(messages: &[WireMessage]) -> Result<ModelPrompt, AiMuxError> {
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        let role = match m.role.as_str() {
            "system" => Role::System,
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "tool" => Role::Tool,
            other => {
                return Err(AiMuxError::InvalidArgument(format!(
                    "unknown message role '{other}' (expected system/user/assistant/tool)"
                )));
            }
        };
        let parts = m.content.iter().map(to_content_part).collect::<Vec<_>>();
        out.push(ModelMessage {
            role,
            content: MessageContent::Parts(parts),
        });
    }
    Ok(ModelPrompt::Messages(out))
}

/// Wire tool → aimux function tool.
fn to_function_tool(t: &WireTool) -> Tool {
    Tool::Function(FunctionTool {
        name: t.name.clone(),
        description: t.description.clone(),
        input_schema: t.parameters.clone(),
        strict: None,
        provider_options: None,
        input_examples: None,
    })
}

/// Wire options → `GenerateTextOptions` (session id + tools included).
pub fn to_generate_options(
    o: &WireOptions,
    session_id: Option<&str>,
) -> Result<GenerateTextOptions, AiMuxError> {
    let response_format = match &o.response_format {
        None => None,
        Some(Value::String(s)) if s == "text" => None,
        Some(Value::Object(obj)) => {
            let schema = obj.get("schema").cloned();
            let name = obj.get("name").and_then(Value::as_str).map(str::to_string);
            let description = obj
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(aimux_core::options::ResponseFormat::Json {
                schema,
                name,
                description,
            })
        }
        Some(other) => {
            return Err(AiMuxError::InvalidArgument(format!(
                "response_format must be \"text\" or an object, got {other}"
            )));
        }
    };

    Ok(GenerateTextOptions {
        max_output_tokens: o.max_output_tokens,
        temperature: o.temperature,
        stop_sequences: o.stop_sequences.clone(),
        top_p: o.top_p,
        tools: o
            .tools
            .as_ref()
            .map(|ts| ts.iter().map(to_function_tool).collect()),
        response_format,
        headers: o.headers.clone(),
        body_overrides: o.body_overrides.clone(),
        max_retries: o.max_retries,
        session_id: session_id.map(str::to_string),
        include_raw_chunks: o.include_raw_chunks,
        ..Default::default()
    })
}

/// Validate and resolve the wire `api_key` field.
///
/// - `None` → `None` (the provider reads its registered env var).
/// - `Some("env:VAR")` → read the environment variable.
/// - any other literal → rejected (plaintext keys must not reach the backend).
pub fn resolve_api_key(spec: Option<&str>) -> Result<Option<String>, AiMuxError> {
    match spec {
        None => Ok(None),
        Some(s) if s.starts_with("env:") => {
            let var = &s[4..];
            if var.is_empty() {
                return Err(AiMuxError::InvalidArgument(
                    "empty env: reference (use env:VAR_NAME)".into(),
                ));
            }
            match std::env::var(var) {
                Ok(v) => Ok(Some(v)),
                Err(_) => Err(AiMuxError::InvalidArgument(format!(
                    "environment variable `{var}` is not set"
                ))),
            }
        }
        Some(_) => Err(AiMuxError::InvalidArgument(
            "plaintext API keys are not accepted — use env:VAR_NAME or leave empty \
             (the provider's registered env var is read automatically)"
                .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_key_env_and_none() {
        // SAFETY: test-only, single-threaded env mutation.
        unsafe { std::env::set_var("AIMUX_WEB_TEST_KEY", "k-123") };
        assert_eq!(
            resolve_api_key(Some("env:AIMUX_WEB_TEST_KEY")).unwrap(),
            Some("k-123".to_string())
        );
        assert_eq!(resolve_api_key(None).unwrap(), None);
        // SAFETY: test-only cleanup.
        unsafe { std::env::remove_var("AIMUX_WEB_TEST_KEY") };
    }

    #[test]
    fn resolve_key_rejects_plaintext_and_missing_env() {
        assert!(resolve_api_key(Some("sk-literal")).is_err());
        assert!(resolve_api_key(Some("env:AIMUX_WEB_MISSING_VAR")).is_err());
        assert!(resolve_api_key(Some("env:")).is_err());
    }

    #[test]
    fn messages_round_trip_with_tool_parts() {
        let msgs = vec![
            WireMessage {
                role: "user".into(),
                content: vec![WireContentPart::Text { text: "hi".into() }],
            },
            WireMessage {
                role: "assistant".into(),
                content: vec![
                    WireContentPart::Text { text: "ok".into() },
                    WireContentPart::ToolCall {
                        tool_call_id: "c1".into(),
                        tool_name: "calc".into(),
                        input: serde_json::json!({"expr": "1+1"}),
                    },
                ],
            },
            WireMessage {
                role: "tool".into(),
                content: vec![WireContentPart::ToolResult {
                    tool_call_id: "c1".into(),
                    result: serde_json::json!({"value": 2}),
                    is_error: None,
                }],
            },
        ];
        let prompt = to_model_prompt(&msgs).unwrap();
        let ModelPrompt::Messages(list) = prompt else {
            panic!("expected messages");
        };
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].role, Role::Assistant);
        assert_eq!(list[2].role, Role::Tool);
    }

    #[test]
    fn unknown_role_is_rejected() {
        let msgs = vec![WireMessage {
            role: "moderator".into(),
            content: vec![WireContentPart::Text { text: "x".into() }],
        }];
        assert!(to_model_prompt(&msgs).is_err());
    }

    #[test]
    fn options_map_tools_and_session() {
        let o = WireOptions {
            temperature: Some(0.5),
            tools: Some(vec![WireTool {
                name: "calc".into(),
                description: Some("calc".into()),
                parameters: serde_json::json!({"type": "object"}),
            }]),
            ..Default::default()
        };
        let g = to_generate_options(&o, Some("sess-1")).unwrap();
        assert_eq!(g.temperature, Some(0.5));
        assert_eq!(g.session_id.as_deref(), Some("sess-1"));
        assert_eq!(g.tools.as_ref().unwrap().len(), 1);
    }
}
