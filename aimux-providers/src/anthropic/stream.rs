//! Shared Anthropic request/streaming core.
//!
//! The standard Anthropic provider ([`crate::anthropic::model`]) and the
//! Anthropic-AWS provider ([`crate::anthropic_aws::model`]) speak the exact
//! same Messages API. The only differences are endpoint, authentication
//! (Bearer/x-api-key vs AWS SigV4) and the wire body encoding (`Json` vs
//! `Bytes`, the latter preventing re-serialization from invalidating the SigV4
//! signature).
//!
//! This module factors out the parts that are identical across both providers:
//! - [`build_anthropic_request`] — serialize the body once, build auth headers
//!   via a closure, and choose the wire encoding.
//! - [`anthropic_generate_core`] — non-streaming send + response parsing.
//! - [`anthropic_stream_core`] — streaming send + the Anthropic SSE event loop.
//! - [`parse_anthropic_content`] — shared content-block → `GenerateContent`
//!   mapping used by the non-streaming path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::AbortSignal;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::Warning;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, TokenUsage, Usage};
use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RequestTimeout, RetryConfig, send_stream_timed, send_timed,
};
use aimux_stream::SseStream;
use serde_json::{Value, json};

use super::convert::parse_stop_reason;
use super::tool_name_mapping::ToolNameMapping;
use super::types::{AnthropicResponse, ContentBlock, StreamEvent};

/// How the request body is sent over the wire.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BodyEncoding {
    /// Send as `HttpBody::Json` — the HTTP layer re-serializes the body value.
    /// Standard Anthropic path.
    Json,
    /// Send as `HttpBody::Bytes` using the *exact* bytes the headers were built
    /// over. AWS SigV4 path — prevents re-serialization from breaking the
    /// signature (the signature is computed over `body_bytes`, and
    /// `body_bytes` is what is sent).
    Bytes,
}

/// Build an Anthropic `HttpRequest`.
///
/// The body is serialized to bytes once so the same bytes are used both for
/// header construction (the `build_headers` closure — needed by SigV4, which
/// signs the request body) and for the wire body in the [`BodyEncoding::Bytes`]
/// path. For [`BodyEncoding::Json`] the closure receives the serialized bytes
/// but the body is sent as a re-serializable `Value` (the signature is not
/// involved, so re-serialization is harmless).
fn build_anthropic_request(
    endpoint: &str,
    body: &serde_json::Value,
    build_headers: &impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError>,
    body_encoding: BodyEncoding,
    abort_signal: Option<AbortSignal>,
    recording_context: Option<aimux_core::recording::RecordingContext>,
) -> Result<HttpRequest, AiMuxError> {
    // Serialize once; the Bytes path sends these exact bytes and the closure
    // signs over them, guaranteeing signature/body agreement.
    let body_bytes = serde_json::to_vec(body).map_err(|e| AiMuxError::JsonParse(e.to_string()))?;
    let headers = build_headers(&body_bytes, endpoint)?;

    let http_body = match body_encoding {
        BodyEncoding::Json => HttpBody::Json(body.clone()),
        BodyEncoding::Bytes => HttpBody::Bytes(body_bytes, "application/json".to_string()),
    };

    Ok(HttpRequest {
        method: HttpMethod::Post,
        url: endpoint.to_string(),
        headers,
        body: http_body,
        abort_signal,
        call_id: recording_context.as_ref().map(|c| c.call_id.clone()),
        recording_context,
    })
}

/// Read a string field, dropping absent / non-string values.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// Process-wide counter backing [`generate_source_id`].
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Unique id for a `Source` derived from a web-search result. Upstream calls
/// `this.generateId()` at the same points.
fn generate_source_id() -> String {
    format!(
        "anthropic-source-{}",
        SOURCE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// `web_search_result` → the camel-cased shape upstream exposes (:1243-1249).
fn map_web_search_result(result: &Value) -> Value {
    json!({
        "url": result.get("url").cloned().unwrap_or(Value::Null),
        "title": result.get("title").cloned().unwrap_or(Value::Null),
        "pageAge": result.get("page_age").cloned().unwrap_or(Value::Null),
        "encryptedContent": result.get("encrypted_content").cloned().unwrap_or(Value::Null),
        "type": result.get("type").cloned().unwrap_or(Value::Null),
    })
}

/// `web_fetch_tool_result.content` → `(result, is_error)` (upstream :1196-1231).
fn map_web_fetch_result(payload: &Value) -> (Value, Option<bool>) {
    if payload.get("type").and_then(|t| t.as_str()) != Some("web_fetch_result") {
        return (
            json!({
                "type": "web_fetch_tool_result_error",
                "errorCode": payload.get("error_code").cloned().unwrap_or(Value::Null),
            }),
            Some(true),
        );
    }
    let inner = payload.get("content").cloned().unwrap_or(Value::Null);
    let source = inner.get("source").cloned().unwrap_or(Value::Null);
    (
        json!({
            "type": "web_fetch_result",
            "url": payload.get("url").cloned().unwrap_or(Value::Null),
            "retrievedAt": payload.get("retrieved_at").cloned().unwrap_or(Value::Null),
            "content": {
                "type": "document",
                "title": inner.get("title").cloned().unwrap_or(Value::Null),
                "citations": inner.get("citations").cloned().unwrap_or(Value::Null),
                "source": {
                    "type": source.get("type").cloned().unwrap_or(Value::Null),
                    "mediaType": source.get("media_type").cloned().unwrap_or(Value::Null),
                    "data": source.get("data").cloned().unwrap_or(Value::Null),
                },
            },
        }),
        None,
    )
}

/// `code_execution_tool_result.content` → `(result, is_error)`
/// (upstream :1281-1320, covering the plain and encrypted result shapes).
fn map_code_execution_result(payload: &Value) -> (Value, Option<bool>) {
    let content = payload.get("content").cloned().unwrap_or(json!([]));
    match payload.get("type").and_then(|t| t.as_str()) {
        Some("code_execution_result") => (
            json!({
                "type": "code_execution_result",
                "stdout": payload.get("stdout").cloned().unwrap_or(Value::Null),
                "stderr": payload.get("stderr").cloned().unwrap_or(Value::Null),
                "return_code": payload.get("return_code").cloned().unwrap_or(Value::Null),
                "content": content,
            }),
            None,
        ),
        Some("encrypted_code_execution_result") => (
            json!({
                "type": "encrypted_code_execution_result",
                "encrypted_stdout": payload.get("encrypted_stdout").cloned()
                    .unwrap_or(Value::Null),
                "stderr": payload.get("stderr").cloned().unwrap_or(Value::Null),
                "return_code": payload.get("return_code").cloned().unwrap_or(Value::Null),
                "content": content,
            }),
            None,
        ),
        _ => (
            json!({
                "type": "code_execution_tool_result_error",
                "errorCode": payload.get("error_code").cloned().unwrap_or(Value::Null),
            }),
            Some(true),
        ),
    }
}

/// `tool_search_tool_result.content` → `(result, is_error)` (upstream :1355-1370).
///
/// The success payload collapses to the tool-reference array itself.
fn map_tool_search_result(payload: &Value) -> (Value, Option<bool>) {
    let refs = payload
        .get("content")
        .and_then(|c| c.as_array())
        .or_else(|| payload.get("tool_references").and_then(|r| r.as_array()));
    match refs {
        Some(refs) => (
            Value::Array(
                refs.iter()
                    .map(|r| {
                        json!({
                            "type": r.get("type").cloned().unwrap_or(Value::Null),
                            "toolName": r.get("tool_name").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect(),
            ),
            None,
        ),
        None => (
            json!({
                "type": "tool_search_tool_result_error",
                "errorCode": payload.get("error_code").cloned().unwrap_or(Value::Null),
            }),
            Some(true),
        ),
    }
}

/// `advisor_tool_result.content` → `(result, is_error)` (upstream :1382-1400).
fn map_advisor_result(payload: &Value) -> (Value, Option<bool>) {
    let stop_reason = payload.get("stop_reason").cloned();
    let with_stop_reason = |mut v: Value| {
        if let Some(sr) = stop_reason.clone()
            && !sr.is_null()
        {
            v["stopReason"] = sr;
        }
        v
    };
    match payload.get("type").and_then(|t| t.as_str()) {
        Some("advisor_result") => (
            with_stop_reason(json!({
                "type": "advisor_result",
                "text": payload.get("text").cloned().unwrap_or(Value::Null),
            })),
            None,
        ),
        Some("advisor_redacted_result") => (
            with_stop_reason(json!({
                "type": "advisor_redacted_result",
                "encryptedContent": payload.get("encrypted_content").cloned()
                    .unwrap_or(Value::Null),
            })),
            None,
        ),
        _ => (
            json!({
                "type": "advisor_tool_result_error",
                "errorCode": payload.get("error_code").cloned().unwrap_or(Value::Null),
            }),
            Some(true),
        ),
    }
}

/// Which `tool_search_*` provider name this call registered.
///
/// Anthropic reports both regex and bm25 searches through one result block, so
/// upstream probes which variant the caller renamed and defaults to regex
/// (:1337-1352).
fn tool_search_provider_name(names: &ToolNameMapping) -> &'static str {
    if names.to_custom_tool_name("tool_search_tool_bm25") != "tool_search_tool_bm25" {
        "tool_search_tool_bm25"
    } else {
        "tool_search_tool_regex"
    }
}

/// Stream parts for a server-tool result block.
///
/// Result blocks arrive complete on `content_block_start`, so the payload
/// mapping is shared with [`parse_anthropic_content`]; only the part type
/// differs. Returns an empty vec for blocks that are not results.
pub(crate) fn stream_parts_for_result_block(
    block: &ContentBlock,
    names: &ToolNameMapping,
    mcp_tool_calls: &HashMap<String, (String, String)>,
) -> Vec<StreamPart> {
    let tool_result =
        |tool_name: String, (result, is_error): (Value, Option<bool>), tool_use_id: &str| {
            StreamPart::ToolResult {
                tool_call_id: tool_use_id.to_string(),
                tool_name,
                result,
                is_error,
                preliminary: None,
                dynamic: None,
                provider_metadata: None,
            }
        };

    match block {
        ContentBlock::WebSearchToolResult {
            tool_use_id,
            content,
        } => {
            let name = names.to_custom_tool_name("web_search").to_string();
            let Some(results) = content.as_array() else {
                return vec![StreamPart::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: name,
                    result: json!({
                        "type": "web_search_tool_result_error",
                        "errorCode": content.get("error_code").cloned().unwrap_or(Value::Null),
                    }),
                    is_error: Some(true),
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                }];
            };
            // The tool result, then one Source per hit — the same pair the
            // non-streaming path emits.
            let mut parts = vec![tool_result(
                name,
                (
                    Value::Array(results.iter().map(map_web_search_result).collect()),
                    None,
                ),
                tool_use_id,
            )];
            parts.extend(results.iter().map(|result| StreamPart::Source {
                id: generate_source_id(),
                source_type: "url".to_string(),
                url: str_field(result, "url"),
                title: str_field(result, "title"),
                provider_metadata: Some(json!({
                    "anthropic": {
                        "pageAge": result.get("page_age").cloned().unwrap_or(Value::Null),
                    }
                })),
            }));
            parts
        }
        ContentBlock::WebFetchToolResult {
            tool_use_id,
            content,
        } => vec![tool_result(
            names.to_custom_tool_name("web_fetch").to_string(),
            map_web_fetch_result(content),
            tool_use_id,
        )],
        ContentBlock::CodeExecutionToolResult {
            tool_use_id,
            content,
        } => vec![tool_result(
            names.to_custom_tool_name("code_execution").to_string(),
            map_code_execution_result(content),
            tool_use_id,
        )],
        ContentBlock::BashCodeExecutionToolResult {
            tool_use_id,
            content,
        }
        | ContentBlock::TextEditorCodeExecutionToolResult {
            tool_use_id,
            content,
        } => vec![tool_result(
            names.to_custom_tool_name("code_execution").to_string(),
            (content.clone(), None),
            tool_use_id,
        )],
        ContentBlock::ToolSearchToolResult {
            tool_use_id,
            content,
        } => vec![tool_result(
            names
                .to_custom_tool_name(tool_search_provider_name(names))
                .to_string(),
            map_tool_search_result(content),
            tool_use_id,
        )],
        ContentBlock::AdvisorToolResult {
            tool_use_id,
            content,
        } => vec![tool_result(
            names.to_custom_tool_name("advisor").to_string(),
            map_advisor_result(content),
            tool_use_id,
        )],
        ContentBlock::McpToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let call = mcp_tool_calls.get(tool_use_id);
            vec![StreamPart::ToolResult {
                tool_call_id: tool_use_id.clone(),
                tool_name: call.map(|(name, _)| name.clone()).unwrap_or_default(),
                result: content.clone(),
                is_error: *is_error,
                preliminary: None,
                dynamic: Some(true),
                provider_metadata: call.map(|(_, server)| {
                    json!({
                        "anthropic": { "type": "mcp-tool-use", "serverName": server }
                    })
                }),
            }]
        }
        _ => Vec::new(),
    }
}

/// Map Anthropic response content blocks into `GenerateContent` items.
///
/// Text / tool_use / thinking / server_tool_use blocks are surfaced, and every
/// server-tool result block becomes a `ToolResult` whose payload is reshaped to
/// match the upstream contract. `web_search` results additionally produce
/// `Source` items, which is how their URLs reach `result.sources`.
///
/// `names` maps Anthropic's wire tool names back to the names the caller used;
/// pass the mapping built from `CallOptions.tools`.
pub(crate) fn parse_anthropic_content(
    blocks: &[ContentBlock],
    names: &ToolNameMapping,
) -> Vec<GenerateContent> {
    let mut content = Vec::new();
    // `mcp_tool_result` inherits the name and server of the `mcp_tool_use` it
    // answers, so index those first (upstream keeps the same lookup table).
    let mut mcp_tool_calls: HashMap<&str, (&str, &str)> = HashMap::new();
    for block in blocks {
        if let ContentBlock::McpToolUse {
            id,
            name,
            server_name,
            ..
        } = block
        {
            mcp_tool_calls.insert(id.as_str(), (name.as_str(), server_name.as_str()));
        }
    }

    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                content.push(GenerateContent::Text {
                    text: text.clone(),
                    provider_metadata: None,
                });
            }
            ContentBlock::ToolUse { id, name, input } => {
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.clone(),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
            }
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                content.push(GenerateContent::Reasoning {
                    text: thinking.clone(),
                    provider_metadata: Some(json!({
                        "anthropic": { "signature": signature }
                    })),
                });
            }
            // Provider-executed (server-side) tool calls are surfaced as tool
            // calls so they round-trip on follow-up turns.
            ContentBlock::ServerToolUse { id, name, input } => {
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.clone(),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
            }
            // MCP tool use — provider-executed + dynamic (upstream :1166-1182).
            ContentBlock::McpToolUse {
                id,
                name,
                input,
                server_name,
            } => {
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.clone(),
                    provider_executed: Some(true),
                    dynamic: Some(true),
                    thought_signature: None,
                    provider_metadata: Some(json!({
                        "anthropic": { "type": "mcp-tool-use", "serverName": server_name }
                    })),
                });
            }
            // Redacted thinking — upstream emits as reasoning with redactedData
            ContentBlock::RedactedThinking { data } => {
                content.push(GenerateContent::Reasoning {
                    text: String::new(),
                    provider_metadata: Some(json!({
                        "anthropic": { "redactedData": data }
                    })),
                });
            }
            // ── Server-tool result blocks → GenerateContent::ToolResult ──
            // Payload shapes mirror the TS `doGenerate` switch one-for-one
            // (anthropic-language-model.ts:1196-1400): keys are camel-cased and
            // optional members normalized, so a caller written against the
            // upstream contract reads the same fields.
            ContentBlock::WebSearchToolResult {
                tool_use_id,
                content: payload,
            } => {
                // Success is an array of results; anything else is the error
                // object (upstream branches on `Array.isArray`).
                match payload.as_array() {
                    Some(results) => {
                        content.push(GenerateContent::ToolResult {
                            tool_call_id: tool_use_id.clone(),
                            tool_name: names.to_custom_tool_name("web_search").to_string(),
                            result: Value::Array(
                                results.iter().map(map_web_search_result).collect(),
                            ),
                            is_error: None,
                            preliminary: None,
                            dynamic: None,
                            provider_metadata: None,
                        });
                        // Each result also becomes a `Source` — that is how the
                        // URLs and titles reach `result.sources`.
                        for result in results {
                            content.push(GenerateContent::Source {
                                id: generate_source_id(),
                                source_type: "url".to_string(),
                                url: str_field(result, "url"),
                                title: str_field(result, "title"),
                                provider_metadata: Some(json!({
                                    "anthropic": {
                                        "pageAge": result.get("page_age").cloned()
                                            .unwrap_or(Value::Null),
                                    }
                                })),
                            });
                        }
                    }
                    None => content.push(GenerateContent::ToolResult {
                        tool_call_id: tool_use_id.clone(),
                        tool_name: names.to_custom_tool_name("web_search").to_string(),
                        result: json!({
                            "type": "web_search_tool_result_error",
                            "errorCode": payload.get("error_code").cloned()
                                .unwrap_or(Value::Null),
                        }),
                        is_error: Some(true),
                        preliminary: None,
                        dynamic: None,
                        provider_metadata: None,
                    }),
                }
            }
            ContentBlock::WebFetchToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_web_fetch_result(payload);
                content.push(GenerateContent::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("web_fetch").to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            ContentBlock::CodeExecutionToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_code_execution_result(payload);
                content.push(GenerateContent::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("code_execution").to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            // Upstream shares one arm for these two and passes `content`
            // through unmapped (anthropic-language-model.ts:1323-1334).
            ContentBlock::BashCodeExecutionToolResult {
                tool_use_id,
                content: payload,
            }
            | ContentBlock::TextEditorCodeExecutionToolResult {
                tool_use_id,
                content: payload,
            } => {
                content.push(GenerateContent::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("code_execution").to_string(),
                    result: payload.clone(),
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            ContentBlock::ToolSearchToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_tool_search_result(payload);
                content.push(GenerateContent::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names
                        .to_custom_tool_name(tool_search_provider_name(names))
                        .to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            ContentBlock::AdvisorToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_advisor_result(payload);
                content.push(GenerateContent::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("advisor").to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }
            // MCP tool result — dynamic, and it inherits the name and metadata
            // of the `mcp_tool_use` block it answers (upstream :1184-1194).
            ContentBlock::McpToolResult {
                tool_use_id,
                content: payload,
                is_error,
            } => {
                let call = mcp_tool_calls.get(tool_use_id.as_str());
                content.push(GenerateContent::ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: call
                        .map(|(name, _)| (*name).to_string())
                        .unwrap_or_default(),
                    result: payload.clone(),
                    is_error: *is_error,
                    preliminary: None,
                    dynamic: Some(true),
                    provider_metadata: call.map(|(_, server)| {
                        json!({
                            "anthropic": { "type": "mcp-tool-use", "serverName": server }
                        })
                    }),
                });
            }
            _ => {}
        }
    }
    content
}

/// Shared non-streaming Anthropic core.
///
/// Builds and sends the request (using `build_headers` for auth and
/// `body_encoding` for the wire body), then parses the `AnthropicResponse` into
/// a `GenerateResult`. The usage breakdown (reasoning / text token split) is the
/// full version, so both the standard and the AWS provider report the same
/// detailed token accounting.
#[allow(clippy::too_many_arguments)] // core plumbing: endpoint/retry/body/warnings/auth/encoding/abort/timeout
pub(crate) async fn anthropic_generate_core(
    endpoint: &str,
    retry_config: RetryConfig,
    body: serde_json::Value,
    warnings: Vec<Warning>,
    build_headers: impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError>,
    body_encoding: BodyEncoding,
    abort_signal: Option<AbortSignal>,
    timeout: Option<RequestTimeout>,
    recording_context: Option<aimux_core::recording::RecordingContext>,
    tool_names: &ToolNameMapping,
) -> Result<GenerateResult, AiMuxError> {
    let request = build_anthropic_request(
        endpoint,
        &body,
        &build_headers,
        body_encoding,
        abort_signal,
        recording_context,
    )?;
    let resp = send_timed(request, retry_config, &DEFAULT_ERROR_STRUCTURE, timeout).await?;

    let data: AnthropicResponse = serde_json::from_slice(&resp.body)?;

    let content = parse_anthropic_content(&data.content, tool_names);

    let finish_reason = data
        .stop_reason
        .as_deref()
        .map(parse_stop_reason)
        .unwrap_or(FinishReason {
            unified: FinishReasonUnified::Other,
            raw: None,
        });

    // RFC-0015 P0-2: fill cache fields + raw; total = input + cache_read +
    // cache_creation (Anthropic's input_tokens excludes cache). Output side
    // breakdown (text/reasoning) comes from output_tokens_details.
    let usage = super::usage::usage_from_anthropic(&data.usage);

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings,
        provider_metadata: None,
        response: ResponseMetadata {
            id: Some(data.id),
            timestamp: None,
            model_id: Some(data.model),
        },
        request_body: Some(body),
        response_headers: None,
    })
}

/// Per-content-block state during streaming.
///
/// Text blocks track whether a `TextStart` has been emitted; tool_use blocks
/// accumulate the `input_json_delta` partial-json fragments so the final
/// `ToolCall` can carry the parsed JSON object; thinking blocks track whether a
/// `ReasoningStart` has been emitted.
enum BlockState {
    Text {
        started: bool,
    },
    ToolUse {
        id: String,
        name: String,
        accumulated_json: String,
    },
    Thinking {
        started: bool,
    },
}

/// Shared Anthropic streaming core.
///
/// Builds and sends the request (using `build_headers` for auth and
/// `body_encoding` for the wire body), then runs the Anthropic SSE event loop
/// to produce a `StreamResult`.
///
/// `build_headers` receives the serialized body bytes and the endpoint URL —
/// the standard path returns a Bearer/x-api-key header set (ignoring the body),
/// the AWS path returns a SigV4-signed header set (signing over the body).
#[allow(clippy::too_many_arguments)] // core plumbing: endpoint/retry/body/warnings/auth/encoding/abort/timeout
pub(crate) async fn anthropic_stream_core(
    endpoint: &str,
    retry_config: RetryConfig,
    body: serde_json::Value,
    warnings: Vec<Warning>,
    build_headers: impl Fn(&[u8], &str) -> Result<Vec<(String, String)>, AiMuxError>,
    body_encoding: BodyEncoding,
    abort_signal: Option<AbortSignal>,
    timeout: Option<RequestTimeout>,
    recording_context: Option<aimux_core::recording::RecordingContext>,
    tool_names: ToolNameMapping,
) -> Result<StreamResult, AiMuxError> {
    let request = build_anthropic_request(
        endpoint,
        &body,
        &build_headers,
        body_encoding,
        abort_signal,
        recording_context,
    )?;
    let resp = send_stream_timed(request, retry_config, &DEFAULT_ERROR_STRUCTURE, timeout).await?;

    let response_headers = resp.headers;
    let sse_stream = SseStream::new(resp.body);

    let stream = async_stream::stream! {
        // First part: StreamStart.
        yield Ok(StreamPart::StreamStart { warnings });

        let mut sse = sse_stream;
        let mut blocks: HashMap<usize, BlockState> = HashMap::new();
        let mut final_usage = Usage::default();
        let mut final_finish_reason: Option<FinishReason> = None;
        let mut response_meta_emitted = false;
        // id → (tool name, server name), so `mcp_tool_result` can inherit them
        // from the `mcp_tool_use` it answers.
        let mut mcp_tool_calls: HashMap<String, (String, String)> = HashMap::new();

        while let Some(event) = sse.next().await {
            match event {
                Ok(sse_event) => {
                    match serde_json::from_str::<StreamEvent>(&sse_event.data) {
                        Ok(StreamEvent::MessageStart { message }) => {
                            if let Some(usage) = &message.usage {
                                // RFC-0015 P0-2: full input side incl. cache
                                // fields + raw (Anthropic reports cache only
                                // in message_start).
                                final_usage = super::usage::usage_from_anthropic(usage);
                            }
                            if !response_meta_emitted {
                                yield Ok(StreamPart::ResponseMetadata {
                                    id: Some(message.id.clone()),
                                    timestamp: None,
                                    model_id: Some(message.model.clone()),
                                });
                                response_meta_emitted = true;
                            }
                        }
                        Ok(StreamEvent::ContentBlockStart { index, content_block }) => {
                            match content_block {
                                ContentBlock::Text { .. } => {
                                    blocks.insert(index, BlockState::Text { started: false });
                                }
                                ContentBlock::Thinking { .. } => {
                                    blocks.insert(index, BlockState::Thinking { started: false });
                                }
                                ContentBlock::ToolUse { id, name, .. } => {
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: name.clone(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    blocks.insert(index, BlockState::ToolUse {
                                        id,
                                        name,
                                        accumulated_json: String::new(),
                                    });
                                }
                                // Server-side tool use — emit as ToolCall.
                                ContentBlock::ServerToolUse { id, name, input } => {
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: id.clone(),
                                        tool_name: tool_names
                                            .to_custom_tool_name(&name)
                                            .to_string(),
                                        input: input.clone(),
                                        provider_executed: Some(true),
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                }
                                // MCP tool use — provider-executed + dynamic.
                                ContentBlock::McpToolUse { id, name, input, server_name } => {
                                    mcp_tool_calls
                                        .insert(id.clone(), (name.clone(), server_name.clone()));
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: id.clone(),
                                        tool_name: name.clone(),
                                        input: input.clone(),
                                        provider_executed: Some(true),
                                        dynamic: Some(true),
                                        thought_signature: None,
                                        provider_metadata: Some(json!({
                                            "anthropic": {
                                                "type": "mcp-tool-use",
                                                "serverName": server_name,
                                            }
                                        })),
                                    });
                                }
                                // Redacted thinking — emit as ReasoningStart.
                                ContentBlock::RedactedThinking { data } => {
                                    let id = index.to_string();
                                    yield Ok(StreamPart::ReasoningStart {
                                        id: id.clone(),
                                        provider_metadata: Some(json!({
                                            "anthropic": { "redactedData": data }
                                        })),
                                    });
                                    blocks.insert(index, BlockState::Thinking { started: true });
                                }
                                // Server-tool result blocks arrive whole on
                                // `content_block_start`, so they reuse the
                                // non-streaming payload mapping. Upstream
                                // mirrors its `doGenerate` switch here too
                                // (anthropic-language-model.ts:1901-2178).
                                other => {
                                    for part in stream_parts_for_result_block(
                                        &other,
                                        &tool_names,
                                        &mcp_tool_calls,
                                    ) {
                                        yield Ok(part);
                                    }
                                }
                            }
                        }
                        Ok(StreamEvent::ContentBlockDelta { index, delta }) => {
                            if let Some(text) = delta.text {
                                // Start the text segment on the first delta. The
                                // text id is the stringified content-block
                                // index, matching the TS SDK.
                                let start_id: Option<String> = match blocks.get_mut(&index) {
                                    Some(BlockState::Text { started: false }) => {
                                        if let Some(BlockState::Text { started }) =
                                            blocks.get_mut(&index)
                                        {
                                            *started = true;
                                        }
                                        Some(index.to_string())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = start_id {
                                    yield Ok(StreamPart::TextStart {
                                        id,
                                        provider_metadata: None,
                                    });
                                }
                                yield Ok(StreamPart::TextDelta {
                                    id: index.to_string(),
                                    delta: text,
                                    provider_metadata: None,
                                });
                            }
                            if let Some(partial) = delta.partial_json {
                                // Accumulate the partial JSON fragment and emit
                                // a ToolInputDelta. Empty fragments (the
                                // leading `input_json_delta` with
                                // `partial_json: ""`) are skipped, matching the
                                // TS SDK.
                                let delta_id: Option<String> = match blocks.get_mut(&index) {
                                    Some(BlockState::ToolUse {
                                        id,
                                        accumulated_json,
                                        ..
                                    }) if !partial.is_empty() => {
                                        accumulated_json.push_str(&partial);
                                        Some(id.clone())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = delta_id {
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id,
                                        delta: partial,
                                        provider_metadata: None,
                                    });
                                }
                            }
                            if let Some(thinking) = delta.thinking {
                                // Start the reasoning segment on the first
                                // thinking delta. The id is the stringified
                                // content-block index, matching the TS SDK.
                                let start_id: Option<String> = match blocks.get_mut(&index) {
                                    Some(BlockState::Thinking { started: false }) => {
                                        if let Some(BlockState::Thinking { started }) =
                                            blocks.get_mut(&index)
                                        {
                                            *started = true;
                                        }
                                        Some(index.to_string())
                                    }
                                    _ => None,
                                };
                                if let Some(id) = start_id {
                                    yield Ok(StreamPart::ReasoningStart {
                                        id,
                                        provider_metadata: None,
                                    });
                                }
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: index.to_string(),
                                    delta: thinking,
                                    provider_metadata: None,
                                });
                            }
                            // #9: signature_delta — emit as ReasoningDelta
                            // with empty delta carrying the signature in
                            // provider_metadata (upstream :2317-2333).
                            if let (Some(sig), true) = (
                                delta.signature,
                                matches!(blocks.get(&index), Some(BlockState::Thinking { .. })),
                            ) {
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: index.to_string(),
                                    delta: String::new(),
                                    provider_metadata: Some(json!({
                                        "anthropic": { "signature": sig }
                                    })),
                                });
                            }
                        }
                        Ok(StreamEvent::ContentBlockStop { index }) => {
                            // Removing the block releases the borrow before any
                            // yield.
                            if let Some(state) = blocks.remove(&index) {
                                match state {
                                    BlockState::Text { started: true } => {
                                        yield Ok(StreamPart::TextEnd {
                                            id: index.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    BlockState::Text { started: false } => {
                                        // Text block with no deltas — nothing to emit.
                                    }
                                    BlockState::Thinking { started: true } => {
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: index.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    BlockState::Thinking { started: false } => {
                                        // Thinking block with no deltas — nothing to emit.
                                    }
                                    BlockState::ToolUse {
                                        id,
                                        name,
                                        accumulated_json,
                                    } => {
                                        yield Ok(StreamPart::ToolInputEnd {
                                            id: id.clone(),
                                            provider_metadata: None,
                                        });
                                        let input: serde_json::Value = if accumulated_json
                                            .is_empty()
                                        {
                                            serde_json::json!({})
                                        } else {
                                            serde_json::from_str(&accumulated_json)
                                                .unwrap_or(serde_json::json!({}))
                                        };
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: id,
                                            tool_name: name,
                                            input,
                                            provider_executed: None,
                                            dynamic: None,
                                            thought_signature: None,
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }
                        }
                        Ok(StreamEvent::MessageDelta { delta, usage }) => {
                            if let Some(reason) = delta.stop_reason {
                                final_finish_reason = Some(parse_stop_reason(&reason));
                            }
                            if let Some(u) = usage {
                                let reasoning_tokens = u
                                    .output_tokens_details
                                    .as_ref()
                                    .and_then(|d| d.thinking_tokens);
                                let output_total = u.output_tokens;
                                let text_tokens = reasoning_tokens
                                    .zip(output_total)
                                    .map(|(r, t)| t.saturating_sub(r));
                                final_usage.output_tokens = TokenUsage {
                                    total: output_total,
                                    text: text_tokens,
                                    reasoning: reasoning_tokens,
                                    ..Default::default()
                                };
                            }
                        }
                        Ok(StreamEvent::MessageStop) => break,
                        Ok(StreamEvent::Error { error }) => {
                            // Surface Anthropic in-stream errors (e.g.
                            // overloaded_error) as a `StreamPart::Error` and
                            // stop the stream, mirroring the TS "forward error
                            // chunks" / "forward overloaded error" behaviour.
                            yield Ok(StreamPart::Error {
                                error: AiMuxError::ApiCall(ApiCallError {
                                    provider_code: error.error_type,
                                    message: error.message,
                                    response_body: Some(sse_event.data.clone()),
                                    ..Default::default()
                                }),
                            });
                            return;
                        }
                        Ok(_) | Err(_) => {}
                    }
                }
                Err(e) => {
                    yield Ok(StreamPart::Error {
                        error: AiMuxError::InvalidResponseData(e.to_string()),
                    });
                    return;
                }
            }
        }

        // Final part: Finish.
        yield Ok(StreamPart::Finish {
            finish_reason: final_finish_reason.unwrap_or(FinishReason {
                unified: FinishReasonUnified::Stop,
                raw: None,
            }),
            usage: final_usage,
            provider_metadata: None,
        });
    };

    Ok(StreamResult {
        stream: Box::pin(stream),
        request_body: Some(body),
        response_headers: Some(response_headers),
    })
}
