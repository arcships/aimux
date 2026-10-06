//! Shared Anthropic request/streaming core.
//!
//! The standard Anthropic provider ([`crate::anthropic::model`]), the
//! Anthropic-AWS provider and Anthropic on Vertex all speak the same Messages
//! API through one [`AnthropicMessagesModel`](super::model::AnthropicMessagesModel);
//! what differs (endpoint, credentials, a SigV4 transport decorator, a body
//! envelope) lives in the model's configuration, not here.
//!
//! This module holds the parts that are identical across all of them:
//! - `anthropic_generate_core` — non-streaming send + response parsing.
//! - `anthropic_stream_core` — streaming send + the Anthropic SSE event loop.
//! - `parse_anthropic_content` — shared content-block → `GenerateContent`
//!   mapping used by the non-streaming path.

use aimux_core::tool::RawToolCall;
use aimux_core::tool::ToolResult;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};
use aimux_core::types::{ProviderMetadata, Warning};
use aimux_provider_utils::HttpRequest;
use serde_json::{Value, json};

use super::config::AnthropicModelConfig;
use super::convert::parse_stop_reason;
use super::options::CANONICAL;
use super::tool_name_mapping::ToolNameMapping;
use super::types::{AnthropicResponse, ContentBlock, StreamErrorData, StreamEvent, ToolCallCaller};

pub(crate) fn anthropic_stream_error(
    error: &StreamErrorData,
    url: &str,
    request_body_values: serde_json::Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
    // Documented Anthropic error types map to their documented statuses; an
    // unknown type carries no status and is not retried (fabricating a 500
    // here was the M3 bug).
    let status_code = match error.error_type.as_deref() {
        Some("rate_limit_error") => Some(429),
        Some("overloaded_error") => Some(529),
        Some("api_error") => Some(500),
        _ => None,
    };
    let data = json!({
        "type": "error",
        "error": {
            "type": error.error_type,
            "message": error.message,
        }
    });
    aimux_provider_utils::stream_error_api_call(
        error.message.clone(),
        error.error_type.clone(),
        status_code,
        &data,
        url,
        request_body_values,
        response_headers,
    )
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
    let mut mapped = json!({
        "url": result["url"],
        "pageAge": result.get("page_age").cloned().unwrap_or(Value::Null),
        "encryptedContent": result["encrypted_content"],
        "type": result["type"],
    });
    if let Some(title) = result.get("title").filter(|title| !title.is_null()) {
        mapped["title"] = title.clone();
    }
    mapped
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

/// Anthropic's 2025 code-execution tool exposes its bash and text-editor
/// operations as distinct wire names, but both belong to the caller's single
/// `code_execution` provider tool.
pub(crate) fn server_tool_provider_name(name: &str) -> &str {
    match name {
        "text_editor_code_execution" | "bash_code_execution" => "code_execution",
        _ => name,
    }
}

/// Finalize a streamed tool-call input accumulated from `input_json_delta`s:
/// empty input normalizes to `"{}"` per the upstream provider, and 2025
/// code-execution variants re-wrap under their wire name so the transcript
/// replays verbatim.
pub(crate) fn finalize_streamed_tool_input(
    mut accumulated_json: String,
    provider_tool_name: Option<&str>,
    provider_tool_input_type: Option<&str>,
) -> String {
    if accumulated_json.is_empty() {
        accumulated_json = "{}".to_string();
    }
    if provider_tool_name == Some("code_execution")
        && let Ok(parsed) = serde_json::from_str::<Value>(&accumulated_json)
    {
        // Only the two known operation names ride through; anything else
        // collapses to the caller's single code_execution tool.
        let wire_name = provider_tool_input_type
            .filter(|name| matches!(*name, "text_editor_code_execution" | "bash_code_execution"))
            .unwrap_or("code_execution");
        accumulated_json = normalized_server_tool_input(wire_name, &parsed).to_string();
    }
    accumulated_json
}

pub(crate) fn normalized_server_tool_input(name: &str, input: &Value) -> Value {
    let input_type = if matches!(name, "text_editor_code_execution" | "bash_code_execution") {
        Some(name)
    } else if name == "code_execution" && input.get("code").is_some() && input.get("type").is_none()
    {
        Some("programmatic-tool-call")
    } else {
        None
    };
    let (Some(input_type), Value::Object(input)) = (input_type, input) else {
        return input.clone();
    };

    let mut normalized = serde_json::Map::new();
    normalized.insert("type".to_string(), Value::String(input_type.to_string()));
    normalized.extend(input.clone());
    Value::Object(normalized)
}

pub(crate) fn initial_tool_input(input: &Value) -> String {
    match input {
        Value::Object(object) if object.is_empty() => String::new(),
        _ => input.to_string(),
    }
}

pub(crate) fn tool_call_caller_metadata(
    caller: Option<&ToolCallCaller>,
    options_name: &str,
) -> Option<ProviderMetadata> {
    let caller = match caller? {
        ToolCallCaller::CodeExecution20250825 { tool_id } => json!({
            "type": "code_execution_20250825",
            "toolId": tool_id,
        }),
        ToolCallCaller::CodeExecution20260120 { tool_id } => json!({
            "type": "code_execution_20260120",
            "toolId": tool_id,
        }),
        ToolCallCaller::Direct => json!({ "type": "direct" }),
    };
    Some(
        provider_namespace(options_name, json!({ "caller": caller }))
            .expect("provider metadata must be an object"),
    )
}

fn toolset_member_input(member_name: &str, input: &Value) -> Value {
    let mut value = json!({ "action": member_name });
    if let Some(input) = input.as_object() {
        value.as_object_mut().unwrap().extend(input.clone());
    }
    value
}

fn tool_call_metadata(
    caller: Option<&ToolCallCaller>,
    toolset_name: Option<&str>,
    options_name: &str,
) -> Option<ProviderMetadata> {
    let mut metadata = tool_call_caller_metadata(caller, options_name);
    if let Some(name) = toolset_name {
        let metadata =
            metadata.get_or_insert_with(|| provider_namespace(options_name, json!({})).unwrap());
        metadata.get_mut(options_name).unwrap()["toolsetName"] = json!(name);
    }
    metadata
}

fn is_tool_search_provider_name(name: &str) -> bool {
    matches!(name, "tool_search_tool_regex" | "tool_search_tool_bm25")
}

/// Resolve the provider tool behind a shared `tool_search_tool_result` block.
///
/// When the matching call is present, its id is authoritative. The name-based
/// fallback preserves Anthropic's deferred-result behavior, where the call may
/// have appeared in an earlier response.
fn tool_search_provider_name<'a>(
    names: &ToolNameMapping,
    provider_name_for_call: Option<&'a str>,
) -> &'a str {
    if let Some(provider_name) = provider_name_for_call
        && is_tool_search_provider_name(provider_name)
    {
        return provider_name;
    }

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
    options_name: &str,
    block: &ContentBlock,
    names: &ToolNameMapping,
    mcp_tool_calls: &HashMap<String, (String, String)>,
    server_tool_calls: &HashMap<String, String>,
) -> Vec<StreamPart> {
    let caller = match block {
        ContentBlock::WebSearchToolResult { caller, .. }
        | ContentBlock::WebFetchToolResult { caller, .. } => caller.as_ref(),
        _ => None,
    };
    let tool_result =
        |tool_name: String, (result, is_error): (Value, Option<bool>), tool_use_id: &str| {
            StreamPart::ToolResult(ToolResult {
                tool_call_id: tool_use_id.to_string(),
                tool_name,
                result,
                is_error,
                preliminary: None,
                dynamic: None,
                provider_metadata: tool_call_caller_metadata(caller, CANONICAL),
            })
        };

    match block {
        ContentBlock::WebSearchToolResult {
            tool_use_id,
            content,
            ..
        } => {
            let name = names.to_custom_tool_name("web_search").to_string();
            let Some(results) = content.as_array() else {
                return vec![StreamPart::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: name,
                    result: json!({
                        "type": "web_search_tool_result_error",
                        "errorCode": content.get("error_code").cloned().unwrap_or(Value::Null),
                    }),
                    is_error: Some(true),
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: tool_call_caller_metadata(caller, CANONICAL),
                })];
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
            parts.extend(results.iter().map(|result| {
                StreamPart::Source(Source::Url {
                    id: generate_source_id(),
                    url: str_field(result, "url").unwrap_or_default(),
                    title: str_field(result, "title"),
                    provider_metadata: Some(
                        provider_namespace(
                            options_name,
                            json!({
                                "pageAge": result.get("page_age").cloned().unwrap_or(Value::Null),
                            }),
                        )
                        .expect("provider metadata must be an object"),
                    ),
                })
            }));
            parts
        }
        ContentBlock::WebFetchToolResult {
            tool_use_id,
            content,
            ..
        } => vec![tool_result(
            names.to_custom_tool_name("web_fetch").to_string(),
            map_web_fetch_result(content),
            tool_use_id,
        )],
        ContentBlock::CodeExecutionToolResult {
            tool_use_id,
            content,
            ..
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
            ..
        } => vec![tool_result(
            names.to_custom_tool_name("code_execution").to_string(),
            (content.clone(), None),
            tool_use_id,
        )],
        ContentBlock::ToolSearchToolResult {
            tool_use_id,
            content,
            ..
        } => vec![tool_result(
            names
                .to_custom_tool_name(tool_search_provider_name(
                    names,
                    server_tool_calls.get(tool_use_id).map(String::as_str),
                ))
                .to_string(),
            map_tool_search_result(content),
            tool_use_id,
        )],
        ContentBlock::AdvisorToolResult {
            tool_use_id,
            content,
            ..
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
            vec![StreamPart::ToolResult(ToolResult {
                tool_call_id: tool_use_id.clone(),
                tool_name: call.map(|(name, _)| name.clone()).unwrap_or_default(),
                result: content.clone(),
                is_error: *is_error,
                preliminary: None,
                dynamic: Some(true),
                provider_metadata: call.map(|(_, server)| {
                    provider_namespace(
                        options_name,
                        json!({ "type": "mcp-tool-use", "serverName": server }),
                    )
                    .expect("provider metadata must be an object")
                }),
            })]
        }
        _ => Vec::new(),
    }
}

fn citation_metadata(citations: &[Value]) -> Option<ProviderMetadata> {
    let citations: Vec<_> = citations
        .iter()
        .filter(|citation| citation["type"] == "web_search_result_location")
        .cloned()
        .collect();
    (!citations.is_empty()).then(|| {
        provider_namespace(CANONICAL, json!({ "citations": citations }))
            .expect("provider metadata must be an object")
    })
}

pub(crate) struct CitationDocument {
    pub title: String,
    pub filename: Option<String>,
    pub media_type: String,
}

fn citation_source(citation: &Value, documents: &[CitationDocument]) -> Option<Source> {
    match citation["type"].as_str()? {
        "web_search_result_location" => Some(Source::Url {
            id: generate_source_id(),
            url: str_field(citation, "url")?,
            title: str_field(citation, "title"),
            provider_metadata: Some(
                provider_namespace(
                    CANONICAL,
                    json!({
                        "citedText": citation["cited_text"],
                        "encryptedIndex": citation["encrypted_index"],
                    }),
                )
                .expect("provider metadata must be an object"),
            ),
        }),
        kind @ ("page_location" | "char_location") => {
            let index = usize::try_from(citation["document_index"].as_u64()?).ok()?;
            let document = documents.get(index)?;
            let metadata = if kind == "page_location" {
                json!({
                    "citedText": citation["cited_text"],
                    "startPageNumber": citation["start_page_number"],
                    "endPageNumber": citation["end_page_number"],
                })
            } else {
                json!({
                    "citedText": citation["cited_text"],
                    "startCharIndex": citation["start_char_index"],
                    "endCharIndex": citation["end_char_index"],
                })
            };
            Some(Source::Document {
                id: generate_source_id(),
                media_type: document.media_type.clone(),
                title: str_field(citation, "document_title")
                    .unwrap_or_else(|| document.title.clone()),
                filename: document.filename.clone(),
                provider_metadata: Some(
                    provider_namespace(CANONICAL, metadata)
                        .expect("provider metadata must be an object"),
                ),
            })
        }
        _ => None,
    }
}

fn web_fetch_document(payload: &Value) -> Option<CitationDocument> {
    (payload["type"] == "web_fetch_result").then(|| CitationDocument {
        title: str_field(&payload["content"], "title")
            .or_else(|| str_field(payload, "url"))
            .unwrap_or_default(),
        filename: None,
        media_type: str_field(&payload["content"]["source"], "media_type").unwrap_or_default(),
    })
}

fn compaction_metadata(signature: Option<&str>) -> ProviderMetadata {
    let mut metadata = json!({ "type": "compaction" });
    if let Some(signature) = signature {
        metadata["signature"] = json!(signature);
    }
    provider_namespace(CANONICAL, metadata).expect("provider metadata must be an object")
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
    _options_name: &str,
    blocks: &[ContentBlock],
    names: &ToolNameMapping,
    uses_json_response_tool: bool,
    mut citation_documents: Vec<CitationDocument>,
) -> Vec<GenerateContent> {
    let options_name = CANONICAL;
    let mut content = Vec::new();
    // Result blocks inherit information from their matching calls. Index the
    // complete response first so ordering does not affect non-stream parsing.
    let mut mcp_tool_calls: HashMap<&str, (&str, &str)> = HashMap::new();
    let mut server_tool_calls: HashMap<&str, &str> = HashMap::new();
    for block in blocks {
        match block {
            ContentBlock::McpToolUse {
                id,
                name,
                server_name,
                ..
            } => {
                mcp_tool_calls.insert(id.as_str(), (name.as_str(), server_name.as_str()));
            }
            ContentBlock::ServerToolUse { id, name, .. } if is_tool_search_provider_name(name) => {
                server_tool_calls.insert(id.as_str(), name.as_str());
            }
            _ => {}
        }
    }

    for block in blocks {
        match block {
            ContentBlock::ContainerUpload { file_id } => {
                content.push(GenerateContent::Custom {
                    kind: "anthropic.container_upload".to_string(),
                    provider_metadata: Some(
                        provider_namespace(CANONICAL, json!({ "fileId": file_id }))
                            .expect("provider metadata must be an object"),
                    ),
                });
            }
            ContentBlock::Text { .. } if uses_json_response_tool => {}
            ContentBlock::Text { text, citations } => {
                content.push(GenerateContent::Text {
                    text: text.clone(),
                    provider_metadata: citation_metadata(citations),
                });
                content.extend(
                    citations
                        .iter()
                        .filter_map(|citation| citation_source(citation, &citation_documents))
                        .map(GenerateContent::Source),
                );
            }
            ContentBlock::Compaction {
                content: Some(text),
                signature,
            } if !text.is_empty() => {
                content.push(GenerateContent::Text {
                    text: text.clone(),
                    provider_metadata: Some(compaction_metadata(signature.as_deref())),
                });
            }
            ContentBlock::ToolUse { name, input, .. }
                if uses_json_response_tool && name == "json" =>
            {
                content.push(GenerateContent::Text {
                    text: input.to_string(),
                    provider_metadata: None,
                });
            }
            ContentBlock::ToolUse {
                id,
                name,
                input,
                toolset_name,
                caller,
            } => {
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id.clone(),
                    tool_name: toolset_name.as_deref().map_or_else(
                        || name.clone(),
                        |name| names.to_custom_tool_name(name).to_string(),
                    ),
                    input: toolset_name.as_ref().map_or_else(
                        || input.to_string(),
                        |_| toolset_member_input(name, input).to_string(),
                    ),
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: tool_call_metadata(
                        caller.as_ref(),
                        toolset_name.as_deref(),
                        options_name,
                    ),
                }));
            }
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                content.push(GenerateContent::Reasoning(ReasoningOutput {
                    text: thinking.clone(),
                    provider_metadata: Some(
                        provider_namespace(options_name, json!({ "signature": signature }))
                            .expect("provider metadata must be an object"),
                    ),
                }));
            }
            // Provider-executed (server-side) tool calls are surfaced as tool
            // calls so they round-trip on follow-up turns.
            ContentBlock::ServerToolUse {
                id,
                name,
                input,
                caller,
            } => {
                let provider_name = server_tool_provider_name(name);
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id.clone(),
                    tool_name: names.to_custom_tool_name(provider_name).to_string(),
                    input: normalized_server_tool_input(name, input).to_string(),
                    provider_executed: Some(true),
                    dynamic: (provider_name == "code_execution"
                        && names.mark_code_execution_dynamic())
                    .then_some(true),
                    provider_metadata: tool_call_caller_metadata(caller.as_ref(), CANONICAL),
                }));
            }
            // MCP tool use — provider-executed + dynamic (upstream :1166-1182).
            ContentBlock::McpToolUse {
                id,
                name,
                input,
                server_name,
            } => {
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.to_string(),
                    provider_executed: Some(true),
                    dynamic: Some(true),
                    provider_metadata: Some(
                        provider_namespace(
                            options_name,
                            json!({ "type": "mcp-tool-use", "serverName": server_name }),
                        )
                        .expect("provider metadata must be an object"),
                    ),
                }));
            }
            // Redacted thinking — upstream emits as reasoning with redactedData
            ContentBlock::RedactedThinking { data } => {
                content.push(GenerateContent::Reasoning(ReasoningOutput {
                    text: String::new(),
                    provider_metadata: Some(
                        provider_namespace(options_name, json!({ "redactedData": data }))
                            .expect("provider metadata must be an object"),
                    ),
                }));
            }
            // ── Server-tool result blocks → GenerateContent::ToolResult ──
            // Payload shapes mirror the TS `doGenerate` switch one-for-one
            // (anthropic-language-model.ts:1196-1400): keys are camel-cased and
            // optional members normalized, so a caller written against the
            // upstream contract reads the same fields.
            ContentBlock::WebSearchToolResult {
                tool_use_id,
                content: payload,
                caller,
            } => {
                // Success is an array of results; anything else is the error
                // object (upstream branches on `Array.isArray`).
                match payload.as_array() {
                    Some(results) => {
                        content.push(GenerateContent::ToolResult(ToolResult {
                            tool_call_id: tool_use_id.clone(),
                            tool_name: names.to_custom_tool_name("web_search").to_string(),
                            result: Value::Array(
                                results.iter().map(map_web_search_result).collect(),
                            ),
                            is_error: None,
                            preliminary: None,
                            dynamic: None,
                            provider_metadata: tool_call_caller_metadata(
                                caller.as_ref(),
                                CANONICAL,
                            ),
                        }));
                        // Each result also becomes a `Source` — that is how the
                        // URLs and titles reach `result.sources`.
                        for result in results {
                            content.push(GenerateContent::Source(Source::Url {
                                id: generate_source_id(),
                                url: str_field(result, "url").unwrap_or_default(),
                                title: str_field(result, "title"),
                                provider_metadata: Some(
                                    provider_namespace(
                                        options_name,
                                        json!({
                                            "pageAge": result.get("page_age").cloned()
                                                .unwrap_or(Value::Null),
                                        }),
                                    )
                                    .expect("provider metadata must be an object"),
                                ),
                            }));
                        }
                    }
                    None => content.push(GenerateContent::ToolResult(ToolResult {
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
                        provider_metadata: tool_call_caller_metadata(caller.as_ref(), CANONICAL),
                    })),
                }
            }
            ContentBlock::WebFetchToolResult {
                tool_use_id,
                content: payload,
                caller,
            } => {
                if let Some(document) = web_fetch_document(payload) {
                    citation_documents.push(document);
                }
                let (result, is_error) = map_web_fetch_result(payload);
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("web_fetch").to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: tool_call_caller_metadata(caller.as_ref(), CANONICAL),
                }));
            }
            ContentBlock::CodeExecutionToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_code_execution_result(payload);
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("code_execution").to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
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
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("code_execution").to_string(),
                    result: payload.clone(),
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
            ContentBlock::ToolSearchToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_tool_search_result(payload);
                let provider_name = tool_search_provider_name(
                    names,
                    server_tool_calls.get(tool_use_id.as_str()).copied(),
                );
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name(provider_name).to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
            ContentBlock::AdvisorToolResult {
                tool_use_id,
                content: payload,
            } => {
                let (result, is_error) = map_advisor_result(payload);
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: names.to_custom_tool_name("advisor").to_string(),
                    result,
                    is_error,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
            // MCP tool result — dynamic, and it inherits the name and metadata
            // of the `mcp_tool_use` block it answers (upstream :1184-1194).
            ContentBlock::McpToolResult {
                tool_use_id,
                content: payload,
                is_error,
            } => {
                let call = mcp_tool_calls.get(tool_use_id.as_str());
                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: tool_use_id.clone(),
                    tool_name: call
                        .map(|(name, _)| (*name).to_string())
                        .unwrap_or_default(),
                    result: payload.clone(),
                    is_error: *is_error,
                    preliminary: None,
                    dynamic: Some(true),
                    provider_metadata: call.map(|(_, server)| {
                        provider_namespace(
                            options_name,
                            json!({ "type": "mcp-tool-use", "serverName": server }),
                        )
                        .expect("provider metadata must be an object")
                    }),
                }));
            }
            _ => {}
        }
    }
    content
}

/// Shared non-streaming Anthropic core.
///
/// Sends `body` through `request` (URL, headers and transport already
/// resolved by the model), then parses the `AnthropicResponse` into a
/// `GenerateResult`. The usage breakdown (reasoning / text token split) is the
/// full version, so every host reports the same detailed token accounting.
/// `config` supplies the providerOptions key the metadata is written under and
/// the host's error shape.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn anthropic_generate_core(
    request: HttpRequest,
    body: serde_json::Value,
    warnings: Vec<Warning>,
    config: &AnthropicModelConfig,
    tool_names: &ToolNameMapping,
    used_custom_options_key: bool,
    uses_json_response_tool: bool,
    citation_documents: Vec<CitationDocument>,
) -> Result<GenerateResult, AiMuxError> {
    let resp = aimux_provider_utils::post_json_to_api(
        request,
        body.clone(),
        aimux_provider_utils::create_json_response_handler(),
        config.failed_response_handler(),
    )
    .await?;

    let response_body = resp.raw_value;
    let data: AnthropicResponse = resp.value;

    let is_json_response_from_tool = uses_json_response_tool
        && data
            .content
            .iter()
            .any(|part| matches!(part, ContentBlock::ToolUse { name, .. } if name == "json"));
    let content = parse_anthropic_content(
        &config.provider_options_name,
        &data.content,
        tool_names,
        uses_json_response_tool,
        citation_documents,
    );

    let mut finish_reason = data
        .stop_reason
        .as_deref()
        .map(parse_stop_reason)
        .unwrap_or(FinishReason {
            unified: FinishReasonUnified::Other,
            raw: None,
        });

    if is_json_response_from_tool && finish_reason.unified == FinishReasonUnified::ToolCalls {
        finish_reason.unified = FinishReasonUnified::Stop;
    }

    // RFC-0015 P0-2: fill cache fields + raw; total = input + cache_read +
    // cache_creation (Anthropic's input_tokens excludes cache). Output side
    // breakdown (text/reasoning) comes from output_tokens_details.
    let usage = super::usage::usage_from_anthropic(&data.usage);

    let mut provider_metadata = super::usage::result_provider_metadata(
        &config.provider_options_name,
        &serde_json::to_value(&data.usage).unwrap_or(Value::Null),
        data.stop_sequence.as_deref(),
        data.container.as_ref(),
        data.context_management.as_ref(),
        used_custom_options_key,
    );
    super::usage::extend_result_metadata(
        &mut provider_metadata,
        data.stop_details.as_ref(),
        data.input_transformations.as_ref(),
        data.safeguard_results.as_ref(),
    );

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings,
        provider_metadata: Some(provider_metadata),
        response: Some(aimux_core::shared::ResponseInfo {
            id: Some(data.id),
            timestamp: None,
            model_id: Some(data.model),
            headers: Some(resp.response_headers),
            body: response_body,
        }),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
    })
}

/// Per-content-block state during streaming.
///
/// Text blocks collect web citations for `TextEnd`; tool-use blocks accumulate
/// partial JSON for the final tool call. Every text/reasoning block starts as
/// soon as its provider content-block-start event arrives.
enum BlockState {
    Text {
        citations: Vec<Value>,
    },
    ToolUse {
        id: String,
        name: String,
        accumulated_json: String,
        provider_executed: Option<bool>,
        dynamic: Option<bool>,
        provider_tool_name: Option<String>,
        provider_tool_input_type: Option<String>,
        provider_metadata: Option<ProviderMetadata>,
        first_delta: bool,
        toolset_member_name: Option<String>,
    },
    Thinking,
}

/// Shared Anthropic streaming core.
///
/// Sends `body` through `request` (URL, headers and transport already
/// resolved by the model), then runs the Anthropic SSE event loop to produce a
/// `StreamResult`. `config` supplies the providerOptions key the metadata is
/// written under and the host's error shape.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn anthropic_stream_core(
    request: HttpRequest,
    body: serde_json::Value,
    warnings: Vec<Warning>,
    config: &AnthropicModelConfig,
    tool_names: ToolNameMapping,
    used_custom_options_key: bool,
    uses_json_response_tool: bool,
    mut citation_documents: Vec<CitationDocument>,
) -> Result<StreamResult, AiMuxError> {
    let endpoint = request.url.clone();
    let options_name = config.provider_options_name.clone();
    let resp = aimux_provider_utils::post_json_to_api(
        request,
        body.clone(),
        aimux_provider_utils::create_event_source_response_handler::<StreamEvent>(),
        config.failed_response_handler(),
    )
    .await?;

    let response_headers = resp.response_headers;
    let mut sse_stream = resp.value;
    let first_event = match sse_stream.next().await {
        Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
        first_event => first_event,
    };
    if let Some(Ok(StreamEvent::Error { error })) = first_event.as_ref() {
        return Err(anthropic_stream_error(
            error,
            &endpoint,
            body.clone(),
            response_headers,
        ));
    }
    let stream_error_url = endpoint;
    let stream_request_body = body.clone();
    let stream_response_headers = response_headers.clone();

    let stream = async_stream::stream! {
        // First part: StreamStart.
        yield Ok(StreamPart::StreamStart { warnings });

        let mut sse = futures::stream::iter(first_event.into_iter()).chain(sse_stream);
        let mut blocks: HashMap<usize, BlockState> = HashMap::new();
        let mut is_json_response_from_tool = false;
        let mut final_usage = Usage::default();
        let mut final_finish_reason: Option<FinishReason> = None;
        // Result-level providerMetadata: the raw usage (`message_start`'s,
        // updated by every `message_delta`'s), the stop sequence, the
        // container and the context-management edits.
        let mut raw_usage = Value::Object(serde_json::Map::new());
        let mut effective_usage = raw_usage.clone();
        let mut stop_sequence: Option<String> = None;
        let mut container: Option<Value> = None;
        let mut context_management: Option<Value> = None;
        let mut stop_details: Option<Value> = None;
        let mut input_transformations: Option<Value> = None;
        let mut safeguard_results: Option<Value> = None;
        let mut response_meta_emitted = false;
        let mut active_message_id: Option<String> = None;
        let mut message_stopped = false;

        // id → (tool name, server name), so `mcp_tool_result` can inherit them
        // from the `mcp_tool_use` it answers.
        let mut mcp_tool_calls: HashMap<String, (String, String)> = HashMap::new();
        // tool_use_id → provider tool name. Both tool-search variants share
        // one result block type, so the id is required to disambiguate aliases.
        let mut server_tool_calls: HashMap<String, String> = HashMap::new();

        while let Some(event) = sse.next().await {
            match event {
                Ok(stream_event) => {
                    match stream_event {
                        StreamEvent::MessageStart { message } => {
                            if let Some(active_id) = &active_message_id {
                                if active_id == &message.id {
                                    continue;
                                }
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::InvalidResponseData(format!(
                                        "Received message_start for message {:?} while message {:?} is still open.",
                                        message.id, active_id,
                                    )),
                                });
                                return;
                            }
                            active_message_id = Some(message.id.clone());
                            input_transformations = message.input_transformations.or(input_transformations);
                            container = message.container.or(container);
                            if let Some(reason) = message.stop_reason.as_deref() {
                                final_finish_reason = Some(parse_stop_reason(reason));
                            }
                            if let Some(usage) = &message.usage {
                                // RFC-0015 P0-2: full input side incl. cache
                                // fields + raw (Anthropic reports cache only
                                // in message_start).
                                final_usage = super::usage::usage_from_anthropic(usage);
                                raw_usage = serde_json::to_value(usage).unwrap_or(raw_usage);
                                effective_usage = raw_usage.clone();
                            }
                            if !response_meta_emitted {
                                yield Ok(StreamPart::ResponseMetadata(ResponseMetadata {
                                    id: Some(message.id.clone()),
                                    timestamp: None,
                                    model_id: Some(message.model.clone()),
                                }));
                                response_meta_emitted = true;
                            }
                        }
                        StreamEvent::ContentBlockStart { index, content_block } => {
                            match content_block {
                                ContentBlock::Text { .. } if uses_json_response_tool => {}
                                ContentBlock::Text { .. } => {
                                    blocks.insert(index, BlockState::Text { citations: Vec::new() });
                                    yield Ok(StreamPart::TextStart { id: index.to_string(), provider_metadata: None });
                                }
                                ContentBlock::Compaction { content, signature } => {
                                    blocks.insert(index, BlockState::Text { citations: Vec::new() });
                                    yield Ok(StreamPart::TextStart { id: index.to_string(), provider_metadata: Some(compaction_metadata(signature.as_deref())) });
                                    if signature.is_some()
                                        && let Some(text) = content.filter(|text| !text.is_empty()) {
                                            yield Ok(StreamPart::TextDelta { id: index.to_string(), delta: text, provider_metadata: None });
                                        }
                                }
                                ContentBlock::Thinking { .. } => {
                                    yield Ok(StreamPart::ReasoningStart { id: index.to_string(), provider_metadata: None });
                                    blocks.insert(
                                        index,
                                        BlockState::Thinking,
                                    );
                                }
                                ContentBlock::ToolUse { name, .. } if uses_json_response_tool && name == "json" => {
                                    is_json_response_from_tool = true;
                                    blocks.insert(index, BlockState::Text { citations: Vec::new() });
                                    yield Ok(StreamPart::TextStart { id: index.to_string(), provider_metadata: None });
                                }
                                ContentBlock::ToolUse {
                                    id,
                                    name,
                                    input,
                                    toolset_name,
                                    caller,
                                } => {
                                    let custom_name = toolset_name.as_deref().map_or_else(
                                        || name.clone(),
                                        |name| tool_names.to_custom_tool_name(name).to_string(),
                                    );
                                    let initial_input = initial_tool_input(&input);
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: custom_name.clone(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    blocks.insert(index, BlockState::ToolUse {
                                        id,
                                        name: custom_name,
                                        first_delta: initial_input.is_empty(),
                                        accumulated_json: initial_input,
                                        provider_executed: None,
                                        dynamic: None,
                                        provider_tool_name: None,
                                        provider_tool_input_type: None,
                                        provider_metadata: tool_call_metadata(caller.as_ref(), toolset_name.as_deref(), CANONICAL),
                                        toolset_member_name: toolset_name.map(|_| name),
                                    });
                                }
                                // Server-side tool use follows the same input
                                // lifecycle as client tools because some code
                                // execution inputs arrive entirely via deltas.
                                ContentBlock::ServerToolUse { id, name, input, caller } => {
                                    if is_tool_search_provider_name(&name) {
                                        server_tool_calls.insert(id.clone(), name.clone());
                                    }
                                    let provider_name = server_tool_provider_name(&name);
                                    let custom_name = tool_names
                                        .to_custom_tool_name(provider_name)
                                        .to_string();
                                    let dynamic = (provider_name == "code_execution"
                                        && tool_names.mark_code_execution_dynamic())
                                    .then_some(true);
                                    let initial_input = initial_tool_input(&input);
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: id.clone(),
                                        tool_name: custom_name.clone(),
                                        provider_executed: Some(true),
                                        dynamic,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    blocks.insert(index, BlockState::ToolUse {
                                        id,
                                        name: custom_name,
                                        first_delta: initial_input.is_empty(),
                                        accumulated_json: initial_input,
                                        provider_executed: Some(true),
                                        dynamic,
                                        provider_tool_name: Some(provider_name.to_string()),
                                        provider_tool_input_type: match name.as_str() {
                                            "text_editor_code_execution" | "bash_code_execution" => {
                                                Some(name)
                                            }
                                            "code_execution" => {
                                                Some("programmatic-tool-call".to_string())
                                            }
                                            _ => None,
                                        },
                                        provider_metadata: tool_call_caller_metadata(caller.as_ref(), CANONICAL),
                                        toolset_member_name: None,
                                    });
                                }
                                // MCP tool use — provider-executed + dynamic.
                                ContentBlock::McpToolUse { id, name, input, server_name } => {
                                    mcp_tool_calls
                                        .insert(id.clone(), (name.clone(), server_name.clone()));
                                    yield Ok(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: id.clone(),
                                        tool_name: name.clone(),
                                        input: input.to_string(),
                                        provider_executed: Some(true),
                                        dynamic: Some(true),
                                        provider_metadata: Some(provider_namespace(CANONICAL, json!({
                                                "type": "mcp-tool-use",
                                                "serverName": server_name,
                                            })).expect("provider metadata must be an object")),
                                    }));
                                }
                                // Redacted thinking — emit as ReasoningStart.
                                ContentBlock::RedactedThinking { data } => {
                                    let id = index.to_string();
                                    yield Ok(StreamPart::ReasoningStart {
                                        id: id.clone(),
                                        provider_metadata: Some(provider_namespace(CANONICAL, json!({ "redactedData": data })).expect("provider metadata must be an object")),
                                    });
                                    blocks.insert(
                                        index,
                                        BlockState::Thinking,
                                    );
                                }
                                // Server-tool result blocks arrive whole on
                                // `content_block_start`, so they reuse the
                                // non-streaming payload mapping. Upstream
                                // mirrors its `doGenerate` switch here too
                                // (anthropic-language-model.ts:1901-2178).
                                other => {
                                    if let ContentBlock::WebFetchToolResult { content, .. } = &other
                                        && let Some(document) = web_fetch_document(content) {
                                        citation_documents.push(document);
                                    }
                                    for part in stream_parts_for_result_block(
                                        CANONICAL,
                                        &other,
                                        &tool_names,
                                        &mcp_tool_calls,
                                        &server_tool_calls,
                                    ) {
                                        yield Ok(part);
                                    }
                                }
                            }
                        }
                        StreamEvent::ContentBlockDelta { index, delta } => {
                            if !uses_json_response_tool && let Some(text) = delta.text {
                                yield Ok(StreamPart::TextDelta {
                                    id: index.to_string(),
                                    delta: text,
                                    provider_metadata: None,
                                });
                            }
                            if let Some(text) = delta.content {
                                yield Ok(StreamPart::TextDelta { id: index.to_string(), delta: text, provider_metadata: None });
                            }
                            if let Some(citation) = delta.citation {
                                if let Some(BlockState::Text { citations }) = blocks.get_mut(&index) {
                                    citations.push(citation.clone());
                                }
                                if let Some(source) = citation_source(&citation, &citation_documents) {
                                    yield Ok(StreamPart::Source(source));
                                }
                            }
                            if let Some(partial) = delta.partial_json {
                                // Accumulate the partial JSON fragment and emit
                                // a ToolInputDelta. Empty fragments (the
                                // leading `input_json_delta` with
                                // `partial_json: ""`) are skipped, matching the
                                // TS SDK.
                                if is_json_response_from_tool && matches!(blocks.get(&index), Some(BlockState::Text { .. })) && !partial.is_empty() {
                                    yield Ok(StreamPart::TextDelta { id: index.to_string(), delta: partial, provider_metadata: None });
                                    continue;
                                }
                                if is_json_response_from_tool { continue; }
                                let delta_event: Option<(String, String)> =
                                    match blocks.get_mut(&index) {
                                        Some(BlockState::ToolUse {
                                                id,
                                                accumulated_json,
                                                provider_tool_input_type,
                                                first_delta,
                                                toolset_member_name,
                                                ..
                                            }) if !partial.is_empty() => {
                                            let emitted_delta = if *first_delta {
                                                if let Some(input_type) = provider_tool_input_type {
                                                    format!(
                                                        "{{\"type\": \"{input_type}\",{}",
                                                        partial.strip_prefix('{').unwrap_or(&partial)
                                                    )
                                                } else {
                                                    partial
                                                }
                                            } else {
                                                partial
                                            };
                                            accumulated_json.push_str(&emitted_delta);
                                            *first_delta = false;
                                            toolset_member_name.is_none().then(|| (id.clone(), emitted_delta))
                                        }
                                        _ => None,
                                    };
                                if let Some((id, delta)) = delta_event {
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id,
                                        delta,
                                        provider_metadata: None,
                                    });
                                }
                            }
                            if let Some(thinking) = delta.thinking {
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: index.to_string(),
                                    delta: thinking,
                                    provider_metadata: None,
                                });
                            }
                            if let Some(sig) = delta.signature
                                && let Some(BlockState::Thinking) = blocks.get(&index) {
                                    yield Ok(StreamPart::ReasoningDelta {
                                        id: index.to_string(), delta: String::new(),
                                        provider_metadata: Some(provider_namespace(CANONICAL, json!({ "signature": sig })).expect("provider metadata must be an object")),
                                    });
                                }
                        }
                        StreamEvent::ContentBlockStop { index } => {
                            // Removing the block releases the borrow before any
                            // yield.
                            if let Some(state) = blocks.remove(&index) {
                                match state {
                                    BlockState::Text { citations } => {
                                        yield Ok(StreamPart::TextEnd {
                                            id: index.to_string(),
                                            provider_metadata: citation_metadata(&citations),
                                        });
                                    }
                                    BlockState::Thinking => {
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: index.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                    BlockState::ToolUse {
                                        id,
                                        name,
                                        mut accumulated_json,
                                        provider_executed,
                                        dynamic,
                                        provider_tool_name,
                                        provider_tool_input_type,
                                        provider_metadata,
                                        toolset_member_name,
                                        ..
                                    } => {
                                        if let Some(member_name) = toolset_member_name {
                                            let parsed = if accumulated_json.is_empty() {
                                                Ok(json!({}))
                                            } else {
                                                serde_json::from_str::<Value>(&accumulated_json)
                                            };
                                            if let Ok(input) = parsed {
                                                accumulated_json = toolset_member_input(&member_name, &input).to_string();
                                            }
                                            yield Ok(StreamPart::ToolInputDelta {
                                                id: id.clone(),
                                                delta: accumulated_json.clone(),
                                                provider_metadata: None,
                                            });
                                        }
                                        yield Ok(StreamPart::ToolInputEnd {
                                            id: id.clone(),
                                            provider_metadata: None,
                                        });
                                        let input =
                                            finalize_streamed_tool_input(
                                                accumulated_json,
                                                provider_tool_name.as_deref(),
                                                provider_tool_input_type.as_deref(),
                                            );
                                        yield Ok(StreamPart::ToolCall(RawToolCall {
                                            tool_call_id: id,
                                            tool_name: name,
                                            input,
                                            provider_executed,
                                            dynamic,
                                            provider_metadata,
                                        }));
                                    }
                                }
                            }
                        }
                        StreamEvent::MessageDelta { delta, usage, context_management: edits, input_transformations: transformations } => {
                            if let Some(reason) = delta.stop_reason {
                                final_finish_reason = Some(parse_stop_reason(&reason));
                            }
                            stop_sequence = delta.stop_sequence;
                            container = delta.container;
                            context_management = edits.or(context_management);
                            stop_details = delta.stop_details;
                            input_transformations = transformations.or(input_transformations);
                            safeguard_results = delta.safeguard_results.or(safeguard_results);
                            if let Some(u) = usage {
                                if let (Value::Object(raw), Ok(Value::Object(update))) =
                                    (&mut raw_usage, serde_json::to_value(&u))
                                {
                                    if let Value::Object(effective) = &mut effective_usage {
                                        effective.extend(update.iter().filter(|(_, value)| !value.is_null()).map(|(key, value)| (key.clone(), value.clone())));
                                    }
                                    raw.extend(update);
                                }
                                if let Ok(usage) = serde_json::from_value(effective_usage.clone()) {
                                    final_usage = super::usage::usage_from_anthropic(&usage);
                                    final_usage.raw = raw_usage.as_object().cloned();
                                }
                            }
                        }
                        StreamEvent::MessageStop => {
                            message_stopped = true;
                            break;
                        },
                        StreamEvent::Error { error } => {
                            yield Ok(StreamPart::Error {
                                error: anthropic_stream_error(
                                    &error,
                                    &stream_error_url,
                                    stream_request_body.clone(),
                                    stream_response_headers.clone(),
                                ),
                            });

                        }
                        _ => {}
                    }
                }
                Err(error) => {
                    let recoverable = error.is_recoverable_stream_error();
                    yield Ok(StreamPart::Error { error });
                    if !recoverable {
                        return;
                    }
                }
            }
        }

        // The provider completes a message explicitly; EOF after an error
        // does not synthesize a finish event.
        if !message_stopped {
            return;
        }
        if is_json_response_from_tool && let Some(reason) = &mut final_finish_reason
            && reason.unified == FinishReasonUnified::ToolCalls {
            reason.unified = FinishReasonUnified::Stop;
        }
        yield Ok(StreamPart::Finish {
            finish_reason: final_finish_reason.unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            }),
            usage: final_usage,
            provider_metadata: Some({
                let mut metadata = super::usage::result_provider_metadata(
                    options_name.as_str(),
                                &raw_usage,
                    stop_sequence.as_deref(),
                    container.as_ref(),
                    context_management.as_ref(),
                    used_custom_options_key,
                );
                super::usage::extend_result_metadata(&mut metadata, stop_details.as_ref(), input_transformations.as_ref(), safeguard_results.as_ref());
                metadata
            }),
        });
    };

    Ok(StreamResult {
        stream: Box::pin(stream),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        response: Some(aimux_core::shared::StreamResponseInfo {
            headers: Some(response_headers),
        }),
    })
}
