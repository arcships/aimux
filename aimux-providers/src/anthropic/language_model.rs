//! `AnthropicLanguageModel` (`anthropic-language-model.ts`): the Messages
//! language model, its request body (`getArgs`), the model capabilities
//! (`getModelCapabilities`), and the response and SSE stream parsing.
//!
//! One model serves every host of the Messages API (`api.anthropic.com`,
//! Claude Platform on AWS, Anthropic on Vertex): it builds the request body,
//! asks its model configuration for the URL, the headers and the transport,
//! sends it and parses the response or the SSE stream. What differs between
//! the hosts (endpoint, credentials, a SigV4 transport decorator, a body
//! envelope) lives in the model's configuration, not here.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use indexmap::IndexSet;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::language_model_message::{LanguageModelMessage, UserPart};
use aimux_core::options::{
    CallOptions, FunctionTool, LanguageModelV4ToolChoice, ResponseFormat, Tool,
};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{RawToolCall, ToolResult};
use aimux_core::types::{
    FinishReason, FinishReasonUnified, NamespacedKind, ProviderMetadata, ReasoningEffort,
    ResponseMetadata, Warning,
};
use aimux_provider_utils::{HttpRequest, Transformer, pipe_through};

use super::api::{
    AnthropicResponse, AnthropicUsage, ContentBlock, DeltaBlock, FallbackModel, MessageDeltaBody,
    MessageStartData, StreamErrorData, StreamEvent, ToolCallCaller,
};
use super::config::AnthropicModelConfig;
use super::convert_to_prompt::{convert_prompt_with_validator, field};
use super::get_cache_control::CacheControlValidator;
use super::language_model_options::{
    CANONICAL, anthropic_options_in, parse_anthropic_option_object,
};
use super::map_stop_reason::parse_stop_reason;
use super::prepare_tools::{AnthropicTool, prepare_tools_with_validator};
use super::tool_name_mapping::ToolNameMapping;
use crate::stream::{Controller, Emit, FINISH_OTHER, StreamContext};

/// One call, ready to send.
struct PreparedCall {
    http: HttpRequest,
    body: Value,
    warnings: Vec<Warning>,
    uses_json_response_tool: bool,
}

/// An Anthropic Messages model (e.g. `claude-sonnet-4-20250514`), created by
/// `provider.messages(id)`.
pub struct AnthropicMessagesModel {
    model_id: String,
    config: AnthropicModelConfig,
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
}

impl AnthropicMessagesModel {
    /// A model of the host the config describes.
    pub(crate) fn with_config(model_id: String, config: AnthropicModelConfig) -> Self {
        Self {
            model_id,
            config,
            generate_id: Arc::new(aimux_provider_utils::generate_id),
        }
    }

    #[must_use]
    pub(crate) fn with_generate_id(
        mut self,
        generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    ) -> Self {
        if let Some(generate_id) = generate_id {
            self.generate_id = generate_id;
        }
        self
    }

    fn citation_documents(options: &CallOptions) -> Vec<CitationDocument> {
        options
            .prompt
            .iter()
            .filter_map(|message| match message {
                LanguageModelMessage::User { content, .. } => Some(content),
                _ => None,
            })
            .flatten()
            .filter_map(|part| match part {
                UserPart::File(file)
                    if matches!(file.media_type.as_str(), "application/pdf" | "text/plain")
                        && file
                            .provider_options
                            .as_ref()
                            .and_then(|options| options.get(CANONICAL))
                            .and_then(|options| options.get("citations"))
                            .and_then(|citations| citations.get("enabled"))
                            .and_then(Value::as_bool)
                            == Some(true) =>
                {
                    Some(CitationDocument {
                        title: file
                            .filename
                            .clone()
                            .unwrap_or_else(|| "Untitled Document".to_string()),
                        filename: file.filename.clone(),
                        media_type: file.media_type.clone(),
                    })
                }
                _ => None,
            })
            .collect()
    }

    fn uses_custom_options(&self, options: &CallOptions) -> bool {
        self.config.provider_options_name != CANONICAL
            && options
                .provider_options
                .as_ref()
                .is_some_and(|value| value.contains_key(&self.config.provider_options_name))
    }

    /// Build the request for one call: the body (host preparation and the
    /// provider's `transform_request_body` applied), its warnings, and the
    /// URL/headers/transport the exchange goes through.
    async fn prepare(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<PreparedCall, AiMuxError> {
        let config = self.config.resolved().await?;
        let built =
            build_request_body_for(&self.model_id, options, stream, &config.request_profile())?;
        let body = config.transform_body(built.body);
        let headers = config
            .request_headers(options.headers.as_ref(), &built.betas, &built.option_betas)
            .await?;
        let http = config.http_request(
            config.messages_url(&self.model_id, stream),
            headers,
            options,
        );
        Ok(PreparedCall {
            http,
            body,
            warnings: built.warnings,
            uses_json_response_tool: built.uses_json_response_tool,
        })
    }
}

#[async_trait]
impl LanguageModel for AnthropicMessagesModel {
    /// `"{name}"`: `"anthropic.messages"` unless the provider was named.
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn supported_urls(&self) -> SupportedUrls {
        self.config.supported_urls.clone()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let call = self.prepare(options, false).await?;
        anthropic_generate_core(
            call.http,
            call.body,
            call.warnings,
            &self.config,
            &ToolNameMapping::new(options.tools.as_deref()),
            self.uses_custom_options(options),
            call.uses_json_response_tool,
            Self::citation_documents(options),
            self.generate_id.clone(),
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let call = self.prepare(options, true).await?;
        anthropic_stream_core(
            call.http,
            call.body,
            call.warnings,
            &self.config,
            options,
            self.uses_custom_options(options),
            call.uses_json_response_tool,
            Self::citation_documents(options),
            self.generate_id.clone(),
        )
        .await
    }
}

// ── request/streaming core ───────────────────────────────────────────────────

pub(crate) fn anthropic_stream_error(
    error: &StreamErrorData,
    url: &str,
    request_body_values: serde_json::Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
    // Upstream's `getAnthropicStreamErrorMetadata` (@ai-sdk/anthropic 4.0.71
    // src/anthropic-language-model.ts:95-121): documented error types map to
    // their statuses and retry flags; an unknown type carries no status and
    // is not retried (fabricating a 500 here was the M3 bug). A status or
    // retry flag in the frame itself wins (`createAnthropicStreamError`,
    // :75-93).
    let (inferred_status, inferred_retryable) = match error.error_type.as_deref() {
        Some("api_error") => (Some(500), Some(true)),
        Some("overloaded_error") => (Some(529), Some(true)),
        Some("rate_limit_error") => (Some(429), Some(true)),
        Some("request_too_large") => (Some(413), Some(false)),
        Some("authentication_error") => (Some(401), Some(false)),
        Some("permission_error") => (Some(403), Some(false)),
        Some("not_found_error") => (Some(404), Some(false)),
        Some("billing_error" | "invalid_request_error") => (Some(400), Some(false)),
        _ => (None, None),
    };
    let status_code = error
        .status_code
        .as_ref()
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok())
        .or(inferred_status);
    let is_retryable = error.is_retryable.flatten().or(inferred_retryable);
    aimux_provider_utils::create_provider_stream_error(
        aimux_provider_utils::ProviderStreamError {
            message: error.message.clone(),
            provider_code: error.error_type.clone(),
            status_code,
            is_retryable,
            data: stream_error_data(error),
        },
        &json!({ "type": "error", "error": parsed_stream_error(error) }),
        url,
        request_body_values,
        response_headers,
    )
}

/// The error as the chunk schema parses it (anthropic-api.ts:1450-1459):
/// its known fields only. A field present as `null` stays, as zod's
/// `nullish()` keeps it and `JSON.stringify` writes it.
fn parsed_stream_error(error: &StreamErrorData) -> Value {
    let mut parsed = serde_json::Map::new();
    if let Some(error_type) = &error.error_type {
        parsed.insert("type".into(), json!(error_type));
    }
    parsed.insert("message".into(), json!(error.message));
    for (key, value) in [
        ("code", error.code.clone()),
        ("statusCode", error.status_code.clone()),
        ("isRetryable", error.is_retryable.map(|flag| json!(flag))),
    ] {
        if let Some(value) = value {
            parsed.insert(key.into(), value);
        }
    }
    Value::Object(parsed)
}

/// `data: 'data' in error ? error.data : error` (anthropic-language-model.ts:91).
fn stream_error_data(error: &StreamErrorData) -> Value {
    error
        .data
        .clone()
        .unwrap_or_else(|| parsed_stream_error(error))
}

/// Read a string field, dropping absent / non-string values.
fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
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
    let mut result = json!({
        "type": "web_fetch_result",
        "url": payload.get("url").cloned().unwrap_or(Value::Null),
        "retrievedAt": payload.get("retrieved_at").cloned().unwrap_or(Value::Null),
        "content": {
            "type": "document",
            "title": inner.get("title").cloned().unwrap_or(Value::Null),
            "source": {
                "type": source.get("type").cloned().unwrap_or(Value::Null),
                "mediaType": source.get("media_type").cloned().unwrap_or(Value::Null),
                "data": source.get("data").cloned().unwrap_or(Value::Null),
            },
        },
    });
    // `citations` is optional upstream: a document without it has no key.
    if let Some(citations) = inner.get("citations").filter(|value| !value.is_null()) {
        result["content"]["citations"] = citations.clone();
    }
    (result, None)
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
/// empty input normalizes to `"{}"`, and a code-execution `{ code }` input
/// without a `type` gains `"type": "programmatic-tool-call"`. Any other input
/// stays the streamed text as is (anthropic-language-model.ts:2638-2657).
pub(crate) fn finalize_streamed_tool_input(
    mut accumulated_json: String,
    provider_tool_name: Option<&str>,
) -> String {
    if accumulated_json.is_empty() {
        accumulated_json = "{}".to_string();
    }
    if provider_tool_name == Some("code_execution")
        && let Ok(parsed) = serde_json::from_str::<Value>(&accumulated_json)
        && !contains_forbidden_prototype(&parsed)
        && parsed.get("code").is_some()
        && parsed.get("type").is_none()
    {
        accumulated_json = normalized_server_tool_input("code_execution", &parsed).to_string();
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

/// What secureJsonParse refuses: a `__proto__` key, or a `constructor` object
/// with a `prototype` key, anywhere in the value.
fn contains_forbidden_prototype(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key("__proto__")
                || object
                    .get("constructor")
                    .and_then(Value::as_object)
                    .is_some_and(|constructor| constructor.contains_key("prototype"))
                || object.values().any(contains_forbidden_prototype)
        }
        Value::Array(items) => items.iter().any(contains_forbidden_prototype),
        _ => false,
    }
}

fn toolset_member_input(member_name: &str, input: &Value) -> Value {
    let mut value = json!({ "action": member_name });
    if let Some(input) = input.as_object() {
        value.as_object_mut().unwrap().extend(input.clone());
    }
    value
}

/// The provider metadata of a tool call: `toolsetName` first, then `caller`,
/// as upstream spreads them (anthropic-language-model.ts:1360-1366 and
/// :2677-2686).
fn tool_call_metadata(
    caller: Option<&ToolCallCaller>,
    toolset_name: Option<&str>,
    options_name: &str,
) -> Option<ProviderMetadata> {
    let Some(name) = toolset_name else {
        return tool_call_caller_metadata(caller, options_name);
    };
    let mut entry = json!({ "toolsetName": name });
    if let Some(caller) = tool_call_caller_metadata(caller, options_name)
        .and_then(|mut metadata| metadata.shift_remove(options_name))
        .and_then(|mut namespace| namespace.remove("caller"))
    {
        entry["caller"] = caller;
    }
    Some(provider_namespace(options_name, entry).expect("provider metadata must be an object"))
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
    generate_id: &(dyn Fn() -> String + Send + Sync),
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
                    id: generate_id(),
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

fn citation_source(
    citation: &Value,
    documents: &[CitationDocument],
    generate_id: &(dyn Fn() -> String + Send + Sync),
) -> Option<Source> {
    match citation["type"].as_str()? {
        "web_search_result_location" => Some(Source::Url {
            id: generate_id(),
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
                id: generate_id(),
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

fn fallback_kind() -> NamespacedKind {
    NamespacedKind::new("anthropic.fallback").expect("anthropic.fallback is namespaced")
}

/// A `fallback` block as the `anthropic.fallback` custom part's metadata
/// (anthropic-language-model.ts:1724-1737, 1976-1989).
fn fallback_metadata(from: &FallbackModel, to: &FallbackModel) -> ProviderMetadata {
    provider_namespace(
        CANONICAL,
        json!({
            "type": "fallback",
            "from": { "model": from.model },
            "to": { "model": to.model },
        }),
    )
    .expect("provider metadata must be an object")
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
    generate_id: &(dyn Fn() -> String + Send + Sync),
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
            ContentBlock::Fallback { from, to } => content.push(GenerateContent::Custom {
                kind: fallback_kind(),
                provider_metadata: Some(fallback_metadata(from, to)),
            }),
            ContentBlock::ContainerUpload { file_id } => {
                content.push(GenerateContent::Custom {
                    kind: NamespacedKind::new("anthropic.container_upload")
                        .expect("anthropic.container_upload is namespaced"),
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
                        .filter_map(|citation| {
                            citation_source(citation, &citation_documents, generate_id)
                        })
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
                                id: generate_id(),
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
    generate_id: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
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
        generate_id.as_ref(),
    );

    let mut finish_reason = data
        .stop_reason
        .as_deref()
        .map(parse_stop_reason)
        .unwrap_or(FINISH_OTHER);

    if is_json_response_from_tool && finish_reason.unified == FinishReasonUnified::ToolCalls {
        finish_reason.unified = FinishReasonUnified::Stop;
    }

    // RFC-0015 P0-2: fill cache fields + raw; total = input + cache_read +
    // cache_creation (Anthropic's input_tokens excludes cache). Output side
    // breakdown (text/reasoning) comes from output_tokens_details.
    let usage = super::convert_usage::usage_from_anthropic(&data.usage);

    let mut provider_metadata = super::message_metadata::result_provider_metadata(
        &config.provider_options_name,
        &serde_json::to_value(&data.usage).unwrap_or(Value::Null),
        data.stop_sequence.as_deref(),
        data.container.as_ref(),
        data.context_management.as_ref(),
        used_custom_options_key,
    );
    super::message_metadata::extend_result_metadata(
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
    options: &CallOptions,
    used_custom_options_key: bool,
    uses_json_response_tool: bool,
    citation_documents: Vec<CitationDocument>,
    generate_id: std::sync::Arc<dyn Fn() -> String + Send + Sync>,
) -> Result<StreamResult, AiMuxError> {
    let tool_names = ToolNameMapping::new(options.tools.as_deref());
    let include_raw_chunks = options.include_raw_chunks == Some(true);
    let endpoint = request.url.clone();
    let options_name = config.provider_options_name.clone();
    let resp = aimux_provider_utils::post_json_to_api(
        request,
        body.clone(),
        aimux_provider_utils::create_event_source_response_handler::<Value>(),
        config.failed_response_handler(),
    )
    .await?;

    let response_headers = resp.response_headers;
    // Each event as the AI SDK's `ParseResult`: the raw JSON when the data
    // parsed as JSON, and the chunk schema's result.
    let mut sse_stream = resp.value.map(move |item| match item {
        Ok(raw) if include_raw_chunks => {
            let event =
                serde_json::from_value::<StreamEvent>(raw.clone()).map_err(AiMuxError::from);
            (Some(raw), event)
        }
        Ok(raw) => (
            None,
            serde_json::from_value::<StreamEvent>(raw).map_err(AiMuxError::from),
        ),
        Err(error) => (None, Err(error)),
    });
    let first_event = match sse_stream.next().await {
        Some((_, Err(error @ AiMuxError::ApiCall(_)))) => return Err(error),
        first_event => first_event,
    };
    if let Some((_, Ok(StreamEvent::Error { error }))) = first_event.as_ref() {
        // The thrown `APICallError` (anthropic-language-model.ts:3150-3160):
        // `statusCode: error.statusCode ?? 500`, `responseBody:
        // JSON.stringify(error.data)`, `isRetryable: error.isRetryable ?? false`.
        let mut thrown = anthropic_stream_error(error, &endpoint, body.clone(), response_headers);
        if let AiMuxError::ApiCall(details) = &mut thrown {
            details.status_code.get_or_insert(500);
            details.response_body = Some(stream_error_data(error).to_string());
        }
        return Err(thrown);
    }

    let raw_usage = Value::Object(serde_json::Map::new());
    let stream = pipe_through(
        futures::stream::iter(first_event).chain(sse_stream),
        AnthropicMessagesStream {
            ctx: StreamContext {
                generate_id,
                ..StreamContext::new(options, warnings, endpoint, &body, &response_headers)
            },
            options_name,
            tool_names,
            used_custom_options_key,
            uses_json_response_tool,
            citation_documents,
            effective_usage: raw_usage.clone(),
            raw_usage,
            ..AnthropicMessagesStream::default()
        },
    );

    Ok(StreamResult {
        stream: Box::pin(stream),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        response: Some(aimux_core::shared::StreamResponseInfo {
            headers: Some(response_headers),
        }),
    })
}

/// The `TransformStream` of `AnthropicMessagesLanguageModel.doStream`.
#[derive(Default)]
pub(super) struct AnthropicMessagesStream {
    ctx: StreamContext,
    options_name: String,
    tool_names: ToolNameMapping,
    used_custom_options_key: bool,
    uses_json_response_tool: bool,
    citation_documents: Vec<CitationDocument>,
    blocks: HashMap<usize, BlockState>,
    is_json_response_from_tool: bool,
    final_finish_reason: Option<FinishReason>,
    /// Result-level providerMetadata: the raw usage (`message_start`'s,
    /// updated by every `message_delta`'s), the stop sequence, the
    /// container and the context-management edits.
    raw_usage: Value,
    /// The usage the finish part reports, kept across the messages of the
    /// stream: `message_start` sets the input side, `message_delta` the rest
    /// (upstream's `usage`, anthropic-language-model.ts:2871-2877 and
    /// :2959-2983).
    effective_usage: Value,
    stop_sequence: Option<String>,
    container: Option<Value>,
    context_management: Option<Value>,
    stop_details: Option<Value>,
    input_transformations: Option<Value>,
    safeguard_results: Option<Value>,
    /// The open message, from `message_start` until its `message_stop`.
    active_message_id: Option<String>,
    /// id → (tool name, server name), so `mcp_tool_result` can inherit them
    /// from the `mcp_tool_use` it answers.
    mcp_tool_calls: HashMap<String, (String, String)>,
    /// tool_use_id → provider tool name. Both tool-search variants share
    /// one result block type, so the id is required to disambiguate aliases.
    server_tool_calls: HashMap<String, String>,
}

#[cfg(test)]
impl AnthropicMessagesStream {
    /// A stream reading its options under `options_name`, everything else at
    /// its start state.
    pub(super) fn with_options_name(options_name: &str) -> Self {
        Self {
            options_name: options_name.into(),
            ..Default::default()
        }
    }
}

impl AnthropicMessagesStream {
    fn message_start(&mut self, message: MessageStartData, controller: &mut Controller) {
        if let Some(active_id) = &self.active_message_id {
            if active_id == &message.id {
                return;
            }
            controller.enqueue(StreamPart::Error {
                error: AiMuxError::invalid_response_data(format!(
                    "Received message_start for message {:?} while message {:?} is still open.",
                    message.id, active_id,
                )),
            });
            controller.terminate();
            return;
        }
        self.active_message_id = Some(message.id.clone());
        self.input_transformations = message
            .input_transformations
            .or(self.input_transformations.take());
        self.container = message.container.or(self.container.take());
        if let Some(reason) = message.stop_reason.as_deref() {
            self.final_finish_reason = Some(parse_stop_reason(reason));
        }
        if let Some(usage) = &message.usage {
            // Only the input side is the new message's; the output side
            // stays until a `message_delta` reports it.
            if !self.effective_usage.is_object() {
                self.effective_usage = json!({});
            }
            if let Value::Object(effective) = &mut self.effective_usage {
                for key in [
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                ] {
                    let value = usage.0.get(key).filter(|value| !value.is_null());
                    effective.insert(key.to_string(), value.cloned().unwrap_or(json!(0)));
                }
            }
            self.raw_usage = usage.0.clone();
        }
        controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
            id: Some(message.id.clone()),
            timestamp: None,
            model_id: Some(message.model.clone()),
        }));
        // Deferred programmatic tool calls arrive whole in `message_start`
        // (anthropic-language-model.ts:2905-2955).
        for part in message.content.iter().flatten() {
            if part["type"] != "tool_use" {
                continue;
            }
            let id = str_field(part, "id").unwrap_or_default();
            let name = str_field(part, "name").unwrap_or_default();
            let input = part
                .get("input")
                .filter(|input| !input.is_null())
                .map_or_else(|| "{}".to_string(), Value::to_string);
            let caller = part
                .get("caller")
                .and_then(|caller| serde_json::from_value::<ToolCallCaller>(caller.clone()).ok());
            controller.tool_input_start(id.clone(), name.clone());
            controller.tool_input_delta(id.clone(), input.clone());
            controller.tool_input_end(id.clone());
            controller.enqueue(StreamPart::ToolCall(RawToolCall {
                tool_call_id: id,
                tool_name: name,
                input,
                provider_executed: None,
                dynamic: None,
                provider_metadata: tool_call_caller_metadata(caller.as_ref(), CANONICAL),
            }));
        }
    }

    fn content_block_start(
        &mut self,
        index: usize,
        content_block: ContentBlock,
        controller: &mut Controller,
    ) {
        match content_block {
            ContentBlock::Text { .. } if self.uses_json_response_tool => {}
            ContentBlock::Text { .. } => {
                self.blocks.insert(
                    index,
                    BlockState::Text {
                        citations: Vec::new(),
                    },
                );
                controller.text_start(index.to_string());
            }
            ContentBlock::Compaction { content, signature } => {
                self.blocks.insert(
                    index,
                    BlockState::Text {
                        citations: Vec::new(),
                    },
                );
                controller.enqueue(StreamPart::TextStart {
                    id: index.to_string(),
                    provider_metadata: Some(compaction_metadata(signature.as_deref())),
                });
                if signature.is_some()
                    && let Some(text) = content.filter(|text| !text.is_empty())
                {
                    controller.text_delta(index.to_string(), text);
                }
            }
            ContentBlock::Thinking { .. } => {
                controller.reasoning_start(index.to_string());
                self.blocks.insert(index, BlockState::Thinking);
            }
            ContentBlock::ToolUse { name, .. }
                if self.uses_json_response_tool && name == "json" =>
            {
                self.is_json_response_from_tool = true;
                self.blocks.insert(
                    index,
                    BlockState::Text {
                        citations: Vec::new(),
                    },
                );
                controller.text_start(index.to_string());
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
                    |name| self.tool_names.to_custom_tool_name(name).to_string(),
                );
                let initial_input = initial_tool_input(&input);
                controller.tool_input_start(id.clone(), custom_name.clone());
                self.blocks.insert(
                    index,
                    BlockState::ToolUse {
                        id,
                        name: custom_name,
                        first_delta: initial_input.is_empty(),
                        accumulated_json: initial_input,
                        provider_executed: None,
                        dynamic: None,
                        provider_tool_name: None,
                        provider_tool_input_type: None,
                        provider_metadata: tool_call_metadata(
                            caller.as_ref(),
                            toolset_name.as_deref(),
                            CANONICAL,
                        ),
                        toolset_member_name: toolset_name.map(|_| name),
                    },
                );
            }
            // Server-side tool use follows the same input
            // lifecycle as client tools because some code
            // execution inputs arrive entirely via deltas.
            ContentBlock::ServerToolUse {
                id,
                name,
                input,
                caller,
            } => {
                if is_tool_search_provider_name(&name) {
                    self.server_tool_calls.insert(id.clone(), name.clone());
                }
                let provider_name = server_tool_provider_name(&name);
                let custom_name = self
                    .tool_names
                    .to_custom_tool_name(provider_name)
                    .to_string();
                let dynamic = (provider_name == "code_execution"
                    && self.tool_names.mark_code_execution_dynamic())
                .then_some(true);
                let initial_input = initial_tool_input(&input);
                controller.enqueue(StreamPart::ToolInputStart {
                    id: id.clone(),
                    tool_name: custom_name.clone(),
                    provider_executed: Some(true),
                    dynamic,
                    title: None,
                    provider_metadata: None,
                });
                self.blocks.insert(
                    index,
                    BlockState::ToolUse {
                        id,
                        name: custom_name,
                        first_delta: initial_input.is_empty(),
                        accumulated_json: initial_input,
                        provider_executed: Some(true),
                        dynamic,
                        provider_tool_name: Some(provider_name.to_string()),
                        provider_tool_input_type: match name.as_str() {
                            "text_editor_code_execution" | "bash_code_execution" => Some(name),
                            "code_execution" => Some("programmatic-tool-call".to_string()),
                            _ => None,
                        },
                        provider_metadata: tool_call_caller_metadata(caller.as_ref(), CANONICAL),
                        toolset_member_name: None,
                    },
                );
            }
            // MCP tool use — provider-executed + dynamic.
            ContentBlock::McpToolUse {
                id,
                name,
                input,
                server_name,
            } => {
                self.mcp_tool_calls
                    .insert(id.clone(), (name.clone(), server_name.clone()));
                controller.enqueue(StreamPart::ToolCall(RawToolCall {
                    tool_call_id: id.clone(),
                    tool_name: name.clone(),
                    input: input.to_string(),
                    provider_executed: Some(true),
                    dynamic: Some(true),
                    provider_metadata: Some(
                        provider_namespace(
                            CANONICAL,
                            json!({
                                "type": "mcp-tool-use",
                                "serverName": server_name,
                            }),
                        )
                        .expect("provider metadata must be an object"),
                    ),
                }));
            }
            // Not a block of its own downstream: no state, so its
            // `content_block_stop` is ignored.
            ContentBlock::Fallback { from, to } => controller.enqueue(StreamPart::Custom {
                kind: fallback_kind(),
                provider_metadata: Some(fallback_metadata(&from, &to)),
            }),
            // Redacted thinking — emit as ReasoningStart.
            ContentBlock::RedactedThinking { data } => {
                let id = index.to_string();
                controller.enqueue(StreamPart::ReasoningStart {
                    id: id.clone(),
                    provider_metadata: Some(
                        provider_namespace(CANONICAL, json!({ "redactedData": data }))
                            .expect("provider metadata must be an object"),
                    ),
                });
                self.blocks.insert(index, BlockState::Thinking);
            }
            // Server-tool result blocks arrive whole on
            // `content_block_start`, so they reuse the
            // non-streaming payload mapping. Upstream
            // mirrors its `doGenerate` switch here too
            // (anthropic-language-model.ts:1901-2178).
            other => {
                if let ContentBlock::WebFetchToolResult { content, .. } = &other
                    && let Some(document) = web_fetch_document(content)
                {
                    self.citation_documents.push(document);
                }
                for part in stream_parts_for_result_block(
                    CANONICAL,
                    &other,
                    &self.tool_names,
                    &self.mcp_tool_calls,
                    &self.server_tool_calls,
                    self.ctx.generate_id.as_ref(),
                ) {
                    controller.enqueue(part);
                }
            }
        }
    }

    fn content_block_delta(
        &mut self,
        index: usize,
        delta: DeltaBlock,
        controller: &mut Controller,
    ) {
        if !self.uses_json_response_tool
            && let Some(text) = delta.text
        {
            controller.text_delta(index.to_string(), text);
        }
        if let Some(text) = delta.content {
            controller.text_delta(index.to_string(), text);
        }
        if let Some(citation) = delta.citation {
            if let Some(BlockState::Text { citations }) = self.blocks.get_mut(&index) {
                citations.push(citation.clone());
            }
            if let Some(source) = citation_source(
                &citation,
                &self.citation_documents,
                self.ctx.generate_id.as_ref(),
            ) {
                controller.enqueue(StreamPart::Source(source));
            }
        }
        if let Some(partial) = delta.partial_json {
            // Accumulate the partial JSON fragment and emit
            // a ToolInputDelta. Empty fragments (the
            // leading `input_json_delta` with
            // `partial_json: ""`) are skipped, matching the
            // TS SDK.
            if self.is_json_response_from_tool
                && matches!(self.blocks.get(&index), Some(BlockState::Text { .. }))
                && !partial.is_empty()
            {
                controller.text_delta(index.to_string(), partial);
                return;
            }
            if self.is_json_response_from_tool {
                return;
            }
            let delta_event: Option<(String, String)> = match self.blocks.get_mut(&index) {
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
                    toolset_member_name
                        .is_none()
                        .then(|| (id.clone(), emitted_delta))
                }
                _ => None,
            };
            if let Some((id, delta)) = delta_event {
                controller.tool_input_delta(id, delta);
            }
        }
        if let Some(thinking) = delta.thinking {
            controller.reasoning_delta(index.to_string(), thinking);
        }
        if let Some(sig) = delta.signature
            && let Some(BlockState::Thinking) = self.blocks.get(&index)
        {
            controller.enqueue(StreamPart::ReasoningDelta {
                id: index.to_string(),
                delta: String::new(),
                provider_metadata: Some(
                    provider_namespace(CANONICAL, json!({ "signature": sig }))
                        .expect("provider metadata must be an object"),
                ),
            });
        }
    }

    fn content_block_stop(&mut self, index: usize, controller: &mut Controller) {
        let Some(state) = self.blocks.remove(&index) else {
            return;
        };
        match state {
            BlockState::Text { citations } => {
                controller.enqueue(StreamPart::TextEnd {
                    id: index.to_string(),
                    provider_metadata: citation_metadata(&citations),
                });
            }
            BlockState::Thinking => {
                controller.reasoning_end(index.to_string());
            }
            BlockState::ToolUse {
                id,
                name,
                mut accumulated_json,
                provider_executed,
                dynamic,
                provider_tool_name,
                provider_metadata,
                toolset_member_name,
                ..
            } => {
                if let Some(member_name) = toolset_member_name {
                    // Upstream parses with secureJsonParse and keeps the raw
                    // input when that fails (anthropic-language-model.ts:2606-2632).
                    let parsed = if accumulated_json.is_empty() {
                        Some(json!({}))
                    } else {
                        serde_json::from_str::<Value>(&accumulated_json)
                            .ok()
                            .filter(|input| !contains_forbidden_prototype(input))
                    };
                    if let Some(input) = parsed {
                        accumulated_json = toolset_member_input(&member_name, &input).to_string();
                    }
                    controller.tool_input_delta(id.clone(), accumulated_json.clone());
                }
                controller.tool_input_end(id.clone());
                let input =
                    finalize_streamed_tool_input(accumulated_json, provider_tool_name.as_deref());
                controller.enqueue(StreamPart::ToolCall(RawToolCall {
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

    fn message_delta(
        &mut self,
        delta: MessageDeltaBody,
        usage: Option<AnthropicUsage>,
        edits: Option<Value>,
        transformations: Option<Value>,
    ) {
        if let Some(reason) = delta.stop_reason {
            self.final_finish_reason = Some(parse_stop_reason(&reason));
        }
        self.stop_sequence = delta.stop_sequence;
        self.container = delta.container;
        self.context_management = edits.or(self.context_management.take());
        self.stop_details = delta.stop_details;
        self.input_transformations = transformations.or(self.input_transformations.take());
        self.safeguard_results = delta.safeguard_results.or(self.safeguard_results.take());
        if let Some(u) = usage
            && let (Value::Object(raw), Ok(Value::Object(update))) =
                (&mut self.raw_usage, serde_json::to_value(&u))
        {
            if let Value::Object(effective) = &mut self.effective_usage {
                effective.extend(
                    update
                        .iter()
                        .filter(|(_, value)| !value.is_null())
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
            }
            raw.extend(update);
        }
    }
}

impl Transformer for AnthropicMessagesStream {
    /// Each event as the AI SDK's `ParseResult`: the raw JSON (kept only when
    /// raw chunks are requested) and the chunk schema's result.
    type Input = (Option<Value>, Result<StreamEvent, AiMuxError>);
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(
        &mut self,
        (raw, event): (Option<Value>, Result<StreamEvent, AiMuxError>),
        controller: &mut Controller,
    ) {
        // Every chunk, valid or not, is forwarded first when raw chunks are
        // requested (anthropic@4.0.71 `anthropic-language-model.ts:1956-1958`);
        // a chunk that is not JSON has no raw value.
        if self.ctx.include_raw_chunks
            && (raw.is_some()
                || event
                    .as_ref()
                    .is_err_and(AiMuxError::is_recoverable_stream_error))
        {
            controller.enqueue(StreamPart::Raw {
                raw_value: raw.unwrap_or(Value::Null),
            });
        }
        let stream_event = match event {
            Ok(stream_event) => stream_event,
            Err(error) => {
                let recoverable = error.is_recoverable_stream_error();
                controller.enqueue(StreamPart::Error { error });
                if !recoverable {
                    controller.terminate();
                }
                return;
            }
        };
        match stream_event {
            StreamEvent::MessageStart { message } => self.message_start(message, controller),
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => self.content_block_start(index, content_block, controller),
            StreamEvent::ContentBlockDelta { index, delta } => {
                self.content_block_delta(index, delta, controller);
            }
            StreamEvent::ContentBlockStop { index } => self.content_block_stop(index, controller),
            StreamEvent::MessageDelta {
                delta,
                usage,
                context_management: edits,
                input_transformations: transformations,
            } => self.message_delta(delta, usage, edits, transformations),
            StreamEvent::MessageStop => {
                // Each message finishes on its own; a later `message_start`
                // opens the next one in the same stream.
                self.active_message_id = None;
                self.finish(controller);
            }
            StreamEvent::Error { error } => {
                controller.enqueue(StreamPart::Error {
                    error: anthropic_stream_error(
                        &error,
                        &self.ctx.url,
                        self.ctx.request_body.clone(),
                        self.ctx.response_headers.clone(),
                    ),
                });
            }
            StreamEvent::Other => {}
        }
    }

    /// The provider completes a message explicitly with `message_stop`; EOF
    /// does not synthesize a finish part.
    fn flush(self, _controller: &mut Controller) {}
}

impl AnthropicMessagesStream {
    /// Emit the `Finish` of the message `message_stop` completed.
    fn finish(&mut self, controller: &mut Controller) {
        if self.is_json_response_from_tool
            && let Some(reason) = &mut self.final_finish_reason
            && reason.unified == FinishReasonUnified::ToolCalls
        {
            reason.unified = FinishReasonUnified::Stop;
        }
        let mut metadata = super::message_metadata::result_provider_metadata(
            self.options_name.as_str(),
            &self.raw_usage,
            self.stop_sequence.as_deref(),
            self.container.as_ref(),
            self.context_management.as_ref(),
            self.used_custom_options_key,
        );
        super::message_metadata::extend_result_metadata(
            &mut metadata,
            self.stop_details.as_ref(),
            self.input_transformations.as_ref(),
            self.safeguard_results.as_ref(),
        );
        let mut usage = super::convert_usage::usage_from_anthropic(&AnthropicUsage(
            self.effective_usage.clone(),
        ));
        usage.raw = self.raw_usage.as_object().cloned();
        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.clone().unwrap_or(FINISH_OTHER),
            usage,
            provider_metadata: Some(metadata),
        });
    }
}

// ── model capabilities & reasoning config ───────────────────────────────────

/// Resolved capabilities for an Anthropic model id, mirroring the TS
/// `getModelCapabilities`.
#[derive(Debug, Clone, Copy)]
pub(super) struct ModelCapabilities {
    pub(super) max_output_tokens: u32,
    pub(super) supports_adaptive_thinking: bool,
    pub(super) rejects_sampling_parameters: bool,
    pub(super) supports_xhigh_effort: bool,
    pub(super) rejects_thinking_disabled_above_high_effort: bool,
    pub(super) rejects_thinking_disabled: bool,
    pub(super) rejects_forced_tool_use: bool,
    pub(super) supports_between_tools_thinking: bool,
    /// Whether the model supports native structured outputs (and therefore
    /// strict tool schemas). Mirrors the TS `supportsStructuredOutput` from
    /// `getModelCapabilities`; `supportsStrictTools` tracks it 1:1 because the
    /// config-level `supportsStrictTools` defaults to `true`.
    pub(super) supports_structured_output: bool,
    /// Mirrors the TS `isKnownModel`. When `false` and the caller did not set
    /// `maxOutputTokens`, a compatibility warning is emitted noting the
    /// default token limit applied.
    pub(super) is_known_model: bool,
}

/// Detect whether a model id refers to a legacy Claude model (claude-instant,
/// claude-2, claude-3 — but *not* `claude-3-haiku`, which is matched earlier).
/// Mirrors the TS regex `/claude-(?:instant(?:-|$)|v?2(?=$|[-.:])|3(?=$|[-.]))/`.
fn is_legacy_claude(model_id: &str) -> bool {
    let rest = match model_id.find("claude-") {
        Some(i) => &model_id[i + "claude-".len()..],
        None => return false,
    };
    if rest == "instant" || rest.starts_with("instant-") {
        return true;
    }
    let two_part = rest.strip_prefix('v').unwrap_or(rest);
    if two_part == "2"
        || two_part.starts_with("2-")
        || two_part.starts_with("2.")
        || two_part.starts_with("2:")
    {
        return true;
    }
    if rest == "3" || rest.starts_with("3-") || rest.starts_with("3.") {
        return true;
    }
    false
}

/// Returns the capabilities for an Anthropic model id, mirroring the TS
/// `getModelCapabilities`. The order of the `contains` checks matters.
pub(super) fn get_model_capabilities(model_id: &str) -> ModelCapabilities {
    fn caps(
        max_output_tokens: u32,
        supports_adaptive_thinking: bool,
        rejects_sampling_parameters: bool,
        supports_xhigh_effort: bool,
        rejects_thinking_disabled_above_high_effort: bool,
        supports_structured_output: bool,
        is_known_model: bool,
    ) -> ModelCapabilities {
        ModelCapabilities {
            max_output_tokens,
            supports_adaptive_thinking,
            rejects_sampling_parameters,
            supports_xhigh_effort,
            rejects_thinking_disabled_above_high_effort,
            rejects_thinking_disabled: false,
            rejects_forced_tool_use: false,
            supports_between_tools_thinking: false,
            supports_structured_output,
            is_known_model,
        }
    }

    if let Some((_, suffix)) = model_id.split_once("claude-opus-5") {
        let mut result = caps(128000, true, true, true, true, true, true);
        result.rejects_thinking_disabled = suffix.starts_with("-5");
        result.rejects_forced_tool_use = result.rejects_thinking_disabled;
        result
    } else if let Some((_, suffix)) = model_id.split_once("claude-fable-5") {
        let mut result = caps(128000, true, true, true, false, true, true);
        result.rejects_thinking_disabled = true;
        result.rejects_forced_tool_use = suffix.starts_with("-1");
        result
    } else if let Some((_, suffix)) = model_id.split_once("claude-sonnet-5") {
        let mut result = caps(128000, true, true, true, false, true, true);
        result.supports_between_tools_thinking = suffix.starts_with("-5");
        result.rejects_thinking_disabled_above_high_effort = result.supports_between_tools_thinking;
        result.rejects_thinking_disabled = result.supports_between_tools_thinking;
        result.rejects_forced_tool_use = result.supports_between_tools_thinking;
        result
    } else if model_id.contains("claude-opus-4-8") || model_id.contains("claude-opus-4-7") {
        caps(128000, true, true, true, false, true, true)
    } else if model_id.contains("claude-sonnet-4-6") || model_id.contains("claude-opus-4-6") {
        caps(128000, true, false, false, false, true, true)
    } else if model_id.contains("claude-sonnet-4-5")
        || model_id.contains("claude-opus-4-5")
        || model_id.contains("claude-haiku-4-5")
    {
        caps(64000, false, false, false, false, true, true)
    } else if model_id.contains("claude-opus-4-1") {
        caps(32000, false, false, false, false, true, true)
    } else if model_id.contains("claude-sonnet-4-") || model_id.contains("claude-sonnet-4@") {
        caps(64000, false, false, false, false, false, true)
    } else if model_id.contains("claude-opus-4-") || model_id.contains("claude-opus-4@") {
        caps(32000, false, false, false, false, false, true)
    } else if model_id.contains("claude-3-haiku") {
        caps(4096, false, false, false, false, false, true)
    } else if is_legacy_claude(model_id) {
        caps(4096, false, false, false, false, false, false)
    } else if model_id.contains("claude-") {
        caps(128000, true, true, true, true, true, false)
    } else {
        caps(4096, false, false, false, false, false, false)
    }
}

/// The Anthropic thinking config + optional effort resolved from a top-level
/// reasoning level, mirroring the TS `resolveAnthropicReasoningConfig` return.
struct ReasoningConfig {
    thinking: Value,
    effort: Option<String>,
}

/// Map a reasoning level to a provider effort string, pushing a compatibility
/// warning when the level maps to a different string. Mirrors the TS
/// `mapReasoningToProviderEffort`.
fn map_reasoning_to_effort(
    reasoning: ReasoningEffort,
    supports_xhigh: bool,
    warnings: &mut Vec<Warning>,
) -> Option<String> {
    let level = reasoning.to_string();
    let mapped = match reasoning {
        ReasoningEffort::Minimal => Some("low"),
        ReasoningEffort::Low => Some("low"),
        ReasoningEffort::Medium => Some("medium"),
        ReasoningEffort::High => Some("high"),
        ReasoningEffort::Xhigh => {
            if supports_xhigh {
                Some("xhigh")
            } else {
                Some("max")
            }
        }
        ReasoningEffort::ProviderDefault | ReasoningEffort::None => None,
    };

    let mapped = match mapped {
        Some(m) => m,
        None => {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".to_string(),
                details: Some(format!(
                    "reasoning \"{level}\" is not supported by this model."
                )),
            });
            return None;
        }
    };

    if mapped != level {
        warnings.push(Warning::Compatibility {
            feature: "reasoning".to_string(),
            details: Some(format!(
                "reasoning \"{level}\" is not directly supported by this model. mapped to effort \"{mapped}\"."
            )),
        });
    }

    Some(mapped.to_string())
}

/// Default reasoning budget percentages (of max output tokens), mirroring the
/// TS `DEFAULT_REASONING_BUDGET_PERCENTAGES`.
fn reasoning_budget_percentage(reasoning: ReasoningEffort) -> Option<f64> {
    match reasoning {
        ReasoningEffort::Minimal => Some(0.02),
        ReasoningEffort::Low => Some(0.10),
        ReasoningEffort::Medium => Some(0.30),
        ReasoningEffort::High => Some(0.60),
        ReasoningEffort::Xhigh => Some(0.90),
        ReasoningEffort::ProviderDefault | ReasoningEffort::None => None,
    }
}

/// Map a reasoning level to an absolute token budget, clamped to a minimum of
/// 1024 and the model's max output tokens. Mirrors the TS
/// `mapReasoningToProviderBudget`.
fn map_reasoning_to_budget(
    reasoning: ReasoningEffort,
    max_output_tokens: u32,
    warnings: &mut Vec<Warning>,
) -> Option<u32> {
    let pct = match reasoning_budget_percentage(reasoning) {
        Some(p) => p,
        None => {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".to_string(),
                details: Some(format!(
                    "reasoning \"{reasoning}\" is not supported by this model."
                )),
            });
            return None;
        }
    };
    let raw = (max_output_tokens as f64 * pct).round() as u32;
    Some(max_output_tokens.min(1024.max(raw)))
}

/// Resolve a top-level reasoning level into an Anthropic thinking config +
/// optional effort, mirroring the TS `resolveAnthropicReasoningConfig`.
///
/// Returns `None` for `ProviderDefault` (no config). `None` reasoning maps to a
/// `disabled` thinking config. Other levels map to adaptive thinking (with an
/// effort) or budget-based `enabled` thinking, depending on
/// `supports_adaptive_thinking`.
fn resolve_anthropic_reasoning_config(
    reasoning: ReasoningEffort,
    supports_adaptive_thinking: bool,
    supports_xhigh_effort: bool,
    max_output_tokens_for_model: u32,
    warnings: &mut Vec<Warning>,
) -> Option<ReasoningConfig> {
    if reasoning == ReasoningEffort::ProviderDefault {
        return None;
    }
    if reasoning == ReasoningEffort::None {
        return Some(ReasoningConfig {
            thinking: json!({ "type": "disabled" }),
            effort: None,
        });
    }

    if supports_adaptive_thinking {
        let effort = map_reasoning_to_effort(reasoning, supports_xhigh_effort, warnings)?;
        return Some(ReasoningConfig {
            thinking: json!({ "type": "adaptive", "display": "summarized" }),
            effort: Some(effort),
        });
    }

    let budget_tokens = map_reasoning_to_budget(reasoning, max_output_tokens_for_model, warnings)?;
    Some(ReasoningConfig {
        thinking: json!({ "type": "enabled", "budgetTokens": budget_tokens }),
        effort: None,
    })
}

/// Result of building an Anthropic request body, including warnings and the
/// beta headers required by the request.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub(crate) uses_json_response_tool: bool,
    pub body: Value,
    pub warnings: Vec<Warning>,
    /// Beta headers (e.g. `code-execution-2025-08-25`, `mcp-client-2025-04-04`)
    /// that must be sent on the request via the `anthropic-beta` header, in
    /// the AI SDK's insertion order: the prompt's, the provider options', then
    /// the tools'.
    pub betas: IndexSet<String>,
    /// `providerOptions.anthropic.anthropicBeta`, which the `anthropic-beta`
    /// header carries after `betas` and the betas of the request headers.
    pub option_betas: Vec<String>,
}

/// What differs between the hosts of the Messages API as far as the request
/// body goes: which providerOptions key the caller's options live under (read
/// in addition to the canonical one) and which tool/output features the host
/// accepts.
#[derive(Debug, Clone)]
pub(crate) struct RequestProfile {
    /// The custom providerOptions key; `anthropic` is always read too.
    pub(crate) options_name: String,
    /// Structured outputs and their beta header (Vertex has neither).
    pub(crate) supports_native_structured_output: bool,
    /// `strict` on tool definitions (Vertex rejects it).
    pub(crate) supports_strict_tools: bool,
}

impl Default for RequestProfile {
    /// The first-party API under its canonical name.
    fn default() -> Self {
        Self {
            options_name: CANONICAL.to_string(),
            supports_native_structured_output: true,
            supports_strict_tools: true,
        }
    }
}

/// Read a value from the Anthropic options of a call (`anthropic` merged with
/// the profile's custom key).
fn anthropic_option(
    options: &Option<SharedProviderOptions>,
    profile: &RequestProfile,
    key: &str,
) -> Option<Value> {
    anthropic_options_in(options.as_ref(), &profile.options_name).and_then(|o| o.get(key).cloned())
}

/// Recursively remove `null`-valued fields from JSON objects (mirroring the
/// TS `JSON.stringify` behaviour of dropping `undefined`). Array elements are
/// preserved as-is. Used to strip absent provider-tool args (e.g. `max_uses`
/// on a web_search tool with no `maxUses`) so the request body matches the
/// TS snapshots.
fn strip_null_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for (_, v) in map.iter_mut() {
                strip_null_fields(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_null_fields(v);
            }
        }
        _ => {}
    }
}

// ── Request-body construction (split into helpers, issue M11) ───────────────

/// Strip temperature / topK / topP when the model rejects sampling parameters
/// (with a compatibility warning per stripped value).
fn strip_anthropic_sampling_params(
    options: &CallOptions,
    model_id: &str,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut temperature = options.temperature;
    for (present, feature) in [
        (options.frequency_penalty.is_some(), "frequencyPenalty"),
        (options.presence_penalty.is_some(), "presencePenalty"),
        (options.seed.is_some(), "seed"),
    ] {
        if present {
            warnings.push(Warning::Unsupported {
                feature: feature.to_string(),
                details: None,
            });
        }
    }
    if let Some(value) = temperature
        && !(0.0..=1.0).contains(&value)
    {
        let (limit, details) = if value > 1.0 {
            (
                1.0,
                format!("{value} exceeds anthropic maximum of 1.0. clamped to 1.0"),
            )
        } else {
            (
                0.0,
                format!("{value} is below anthropic minimum of 0. clamped to 0"),
            )
        };
        temperature = Some(limit);
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some(details),
        });
    }
    let mut top_p = options.top_p;
    let mut top_k = options.top_k;

    if caps.rejects_sampling_parameters {
        if temperature.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "temperature".to_string(),
                details: Some(format!(
                    "temperature is not supported by {model_id} and will be ignored"
                )),
            });
            temperature = None;
        }
        if top_k.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: Some(format!(
                    "topK is not supported by {model_id} and will be ignored"
                )),
            });
            top_k = None;
        }
        if top_p.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "topP".to_string(),
                details: Some(format!(
                    "topP is not supported by {model_id} and will be ignored"
                )),
            });
            top_p = None;
        }
    }

    (temperature, top_p, top_k)
}

/// Resolve thinking config + optional effort from provider options and the
/// top-level `reasoning` level.
///
/// `providerOptions.anthropic.thinking` / `.effort` take precedence over the
/// top-level `reasoning`; the top-level mapping only runs when `effort` is not
/// already set by provider options (TS L426-445). Also lowers xhigh/max to
/// high when the model rejects disabling thinking above high effort
/// (TS L451-464).
fn resolve_anthropic_thinking(
    model_id: &str,
    options: &CallOptions,
    profile: &RequestProfile,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) -> (Option<Value>, Option<String>) {
    let mut thinking_config: Option<Value> =
        anthropic_option(&options.provider_options, profile, "thinking");
    let mut effort: Option<String> = anthropic_option(&options.provider_options, profile, "effort")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));

    if let Some(reasoning) = options.reasoning
        && reasoning.is_custom()
        && effort.is_none()
        && let Some(rc) = resolve_anthropic_reasoning_config(
            reasoning,
            caps.supports_adaptive_thinking,
            caps.supports_xhigh_effort,
            caps.max_output_tokens,
            warnings,
        )
    {
        if thinking_config.is_none() {
            thinking_config = Some(rc.thinking);
        }
        if let Some(eff) = rc.effort {
            let is_disabled = thinking_config
                .as_ref()
                .and_then(|t| t.get("type"))
                .and_then(|t| t.as_str())
                == Some("disabled");
            if !is_disabled {
                effort = Some(eff);
            }
        }
    }

    if options.reasoning == Some(ReasoningEffort::None)
        && anthropic_option(&options.provider_options, profile, "thinking").is_none()
        && anthropic_option(&options.provider_options, profile, "effort").is_none()
        && caps.rejects_thinking_disabled
    {
        if caps.supports_between_tools_thinking {
            thinking_config = Some(json!({ "type": "between_tools" }));
        } else {
            thinking_config = None;
            effort = Some("low".to_string());
            warnings.push(Warning::Compatibility { feature: "reasoning".to_string(), details: Some(format!("reasoning 'none' is not supported by {model_id}; it always uses adaptive thinking. Using effort 'low' to minimize thinking instead.")) });
        }
    }

    if caps.rejects_thinking_disabled {
        match thinking_config
            .as_ref()
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
        {
            Some("disabled") => {
                let details = if caps.supports_between_tools_thinking {
                    thinking_config = Some(json!({ "type": "between_tools" }));
                    format!(
                        "thinking cannot be disabled for {model_id}. Using 'between_tools' thinking, the lowest thinking setting, instead."
                    )
                } else {
                    thinking_config = None;
                    format!(
                        "thinking cannot be disabled for {model_id}; it always uses adaptive thinking. The thinking setting has been removed. Lower 'effort' to reduce thinking."
                    )
                };
                warnings.push(Warning::Unsupported {
                    feature: "providerOptions.anthropic.thinking".to_string(),
                    details: Some(details),
                });
            }
            Some("enabled") => {
                thinking_config = Some(json!({ "type": "adaptive" }));
                warnings.push(Warning::Unsupported { feature: "providerOptions.anthropic.thinking".to_string(), details: Some(format!("budget-based thinking is not supported by {model_id}; it always uses adaptive thinking. Using adaptive thinking instead. Use 'effort' to control how much the model thinks.")) });
            }
            _ => {}
        }
    }
    if thinking_config
        .as_ref()
        .and_then(|v| v.get("type"))
        .and_then(Value::as_str)
        == Some("between_tools")
        && matches!(effort.as_deref(), Some("xhigh" | "max"))
    {
        warnings.push(Warning::Unsupported { feature: "providerOptions.anthropic.effort".to_string(), details: Some(format!("effort '{}' is not supported with 'between_tools' thinking. The effort has been lowered to 'high'.", effort.as_deref().unwrap_or_default())) });
        effort = Some("high".to_string());
    }

    // Newer models only allow disabling thinking at effort ≤ high; lower the
    // effort to 'high' with a warning (TS L451-464).
    if caps.rejects_thinking_disabled_above_high_effort {
        let is_disabled = thinking_config
            .as_ref()
            .and_then(|t| t.get("type"))
            .and_then(|t| t.as_str())
            == Some("disabled");
        if is_disabled && (effort.as_deref() == Some("xhigh") || effort.as_deref() == Some("max")) {
            warnings.push(Warning::Unsupported {
                feature: "providerOptions.anthropic.effort".to_string(),
                details: Some(format!(
                    "effort '{}' is not supported by {} when thinking is disabled. The effort has been lowered to 'high'.",
                    effort.as_deref().unwrap_or(""),
                    model_id
                )),
            });
            effort = Some("high".to_string());
        }
    }

    (thinking_config, effort)
}

/// Derived fields from the resolved thinking config: the type string, whether
/// thinking must be forwarded (enabled/adaptive/disabled — some models default
/// thinking on, so omitting `disabled` would leave it enabled), the budget
/// (enabled only), and the display value (adaptive only).
fn derive_anthropic_thinking(
    thinking_config: &Option<Value>,
) -> (Option<String>, bool, Option<u32>, Option<Value>) {
    let thinking_type: Option<String> = thinking_config
        .as_ref()
        .and_then(|t| t.get("type"))
        .and_then(|t| t.as_str())
        .map(std::string::ToString::to_string);

    let is_thinking = matches!(
        thinking_type.as_deref(),
        Some("enabled" | "adaptive" | "between_tools")
    );
    let send_thinking = is_thinking || thinking_type.as_deref() == Some("disabled");

    let thinking_budget: Option<u32> = if thinking_type.as_deref() == Some("enabled") {
        thinking_config
            .as_ref()
            .and_then(|t| t.get("budgetTokens"))
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as u32)
    } else {
        None
    };
    let thinking_display: Option<Value> = if thinking_type.as_deref() == Some("adaptive") {
        thinking_config
            .as_ref()
            .and_then(|t| t.get("display"))
            .cloned()
    } else {
        None
    };

    (
        thinking_type,
        send_thinking,
        thinking_budget,
        thinking_display,
    )
}

/// Insert the `thinking` / `output_config` fields into the body.
fn insert_anthropic_thinking(
    body: &mut Value,
    thinking_type: &Option<String>,
    send_thinking: bool,
    thinking_budget: &Option<u32>,
    thinking_display: &Option<Value>,
    effort: &Option<String>,
) {
    if send_thinking && let Some(tt) = thinking_type {
        let mut thinking_obj = json!({ "type": tt });
        if let Some(b) = thinking_budget {
            thinking_obj["budget_tokens"] = json!(b);
        }
        if let Some(d) = thinking_display {
            thinking_obj["display"] = d.clone();
        }
        body["thinking"] = thinking_obj;
    }

    if let Some(eff) = effort {
        body["output_config"] = json!({ "effort": eff });
    }
}

/// Thinking-enabled post-processing (TS L651-696): default budget warning,
/// sampling-parameter stripping, and `max_tokens` adjustment. Returns the
/// adjusted `max_tokens` (unchanged when thinking is not enabled).
#[allow(clippy::too_many_arguments)]
fn apply_anthropic_thinking_post_processing(
    body: &mut Value,
    thinking_type: &Option<String>,
    thinking_budget: &mut Option<u32>,
    max_tokens: u32,
    temperature: &mut Option<f64>,
    top_k: &mut Option<f64>,
    top_p: &mut Option<f64>,
    warnings: &mut Vec<Warning>,
) -> u32 {
    let is_thinking = matches!(
        thinking_type.as_deref(),
        Some("enabled" | "adaptive" | "between_tools")
    );
    if !is_thinking {
        return max_tokens;
    }

    if thinking_type.as_deref() == Some("enabled") && thinking_budget.is_none() {
        warnings.push(Warning::Compatibility {
            feature: "extended thinking".to_string(),
            details: Some(
                "thinking budget is required when thinking is enabled. using default budget of 1024 tokens.".to_string(),
            ),
        });
        *thinking_budget = Some(1024);
        // Mirrors the original implementation: the default budget must also be
        // written into the `thinking` body field, not just added to max_tokens
        // (audit finding — default budget was dropped during the M11 split).
        if let Some(thinking_obj) = body.get_mut("thinking") {
            thinking_obj["budget_tokens"] = json!(1024);
        }
    }

    if temperature.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "temperature".to_string(),
            details: Some("temperature is not supported when thinking is enabled".to_string()),
        });
        *temperature = None;
    }
    if top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: Some("topK is not supported when thinking is enabled".to_string()),
        });
        *top_k = None;
    }
    if top_p.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topP".to_string(),
            details: Some("topP is not supported when thinking is enabled".to_string()),
        });
        *top_p = None;
    }

    max_tokens.saturating_add(thinking_budget.unwrap_or(0))
}

/// Insert the surviving sampling params + stop sequences into the body.
fn insert_anthropic_sampling(
    body: &mut Value,
    temperature: Option<f64>,
    top_p: Option<f64>,
    top_k: Option<f64>,
    options: &CallOptions,
) {
    if let Some(temp) = temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
    if let Some(k) = top_k {
        body["top_k"] = json!(k);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop_sequences"] = json!(stop);
    }
}

/// Map user tools to `tools` / `tool_choice`, delegating to
/// `prepare_tools_with_provider` (provider-defined tools, required beta
/// headers, and tool warnings).
#[allow(clippy::too_many_arguments)]
fn apply_anthropic_tools(
    body: &mut Value,
    options: &CallOptions,
    profile: &RequestProfile,
    stream: bool,
    json_schema: Option<&Value>,
    caps: &ModelCapabilities,
    betas: &mut IndexSet<String>,
    warnings: &mut Vec<Warning>,
    cache_validator: &mut CacheControlValidator,
) -> Result<(), AiMuxError> {
    let disable_parallel_tool_use = json_schema.is_some()
        || anthropic_option(&options.provider_options, profile, "disableParallelToolUse")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let default_eager_input_streaming = stream
        && anthropic_option(&options.provider_options, profile, "toolStreaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

    let mut anthropic_tools: Vec<AnthropicTool> = match &options.tools {
        Some(tools) => tools
            .iter()
            .map(|t| match t {
                Tool::Function(ft) => AnthropicTool::Function(ft.clone()),
                Tool::Provider(pt) => AnthropicTool::Provider {
                    id: pt.id.to_string(),
                    name: pt.name.clone(),
                    args: pt.args.clone(),
                },
            })
            .collect(),
        None => Vec::new(),
    };

    if let Some(schema) = json_schema {
        anthropic_tools.push(AnthropicTool::Function(
            FunctionTool::new("json", schema.clone())
                .with_description("Respond with a JSON object."),
        ));
    }
    let requested_choice = if json_schema.is_some() {
        Some(&LanguageModelV4ToolChoice::Required)
    } else {
        options.tool_choice.as_ref()
    };
    let tool_choice = if caps.rejects_forced_tool_use
        && !anthropic_tools.is_empty()
        && matches!(
            requested_choice,
            Some(LanguageModelV4ToolChoice::Required | LanguageModelV4ToolChoice::Tool { .. })
        ) {
        warnings.push(Warning::Unsupported {
            feature: "toolChoice".to_string(),
            details: Some(match requested_choice {
                Some(LanguageModelV4ToolChoice::Required) => "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.".to_string(),
                Some(LanguageModelV4ToolChoice::Tool { tool_name }) => format!("toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the '{tool_name}' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made."),
                _ => unreachable!(),
            }),
        });
        Some(&LanguageModelV4ToolChoice::Auto)
    } else {
        requested_choice
    };
    let prepared = prepare_tools_with_validator(
        if options.tools.is_some() || json_schema.is_some() {
            Some(&anthropic_tools)
        } else {
            None
        },
        tool_choice,
        disable_parallel_tool_use,
        json_schema.is_none()
            && caps.supports_structured_output
            && profile.supports_native_structured_output,
        caps.supports_structured_output && profile.supports_strict_tools,
        default_eager_input_streaming,
        &profile.options_name,
        cache_validator,
    )?;

    warnings.extend(prepared.tool_warnings);
    betas.extend(prepared.betas);

    if let Some(tool_defs) = prepared.tools {
        // Strip null-valued fields from tool definitions so the serialized
        // request body matches the TS behaviour (JSON.stringify drops
        // `undefined` args on provider-defined tools such as web_search).
        let mut defs = tool_defs;
        if caps.rejects_forced_tool_use
            && let Some(LanguageModelV4ToolChoice::Tool { tool_name }) = requested_choice
        {
            defs.retain(|tool| {
                tool.get("name").and_then(Value::as_str) == Some(tool_name.as_str())
            });
        }
        for def in defs.iter_mut() {
            strip_null_fields(def);
        }
        body["tools"] = json!(defs);
    }
    if let Some(mut tool_choice) = prepared.tool_choice {
        if json_schema.is_none()
            && let Some(value) =
                anthropic_option(&options.provider_options, profile, "disableParallelToolUse")
                    .filter(Value::is_boolean)
        {
            tool_choice["disable_parallel_tool_use"] = value;
        }
        body["tool_choice"] = tool_choice;
    }
    Ok(())
}

/// `providerOptions.anthropic.mcpServers` → `mcp_servers` (its beta is
/// added by [`provider_option_betas`]).
fn append_anthropic_mcp_servers(body: &mut Value, options: &CallOptions, profile: &RequestProfile) {
    let Some(mcp) = anthropic_option(&options.provider_options, profile, "mcpServers") else {
        return;
    };
    let Some(arr) = mcp.as_array() else {
        return;
    };
    if arr.is_empty() {
        return;
    }

    let mapped: Vec<Value> = arr
        .iter()
        .map(|server| {
            let mut o = Map::new();
            if let Some(t) = server.get("type") {
                o.insert("type".to_string(), t.clone());
            }
            if let Some(n) = server.get("name") {
                o.insert("name".to_string(), n.clone());
            }
            if let Some(u) = server.get("url") {
                o.insert("url".to_string(), u.clone());
            }
            if let Some(at) = server.get("authorizationToken") {
                o.insert("authorization_token".to_string(), at.clone());
            }
            if let Some(tc) = server
                .get("toolConfiguration")
                .filter(|value| !value.is_null())
            {
                let mut tc_obj = Map::new();
                if let Some(at) = tc.get("allowedTools") {
                    tc_obj.insert("allowed_tools".to_string(), at.clone());
                }
                if let Some(en) = tc.get("enabled") {
                    tc_obj.insert("enabled".to_string(), en.clone());
                }
                o.insert("tool_configuration".to_string(), Value::Object(tc_obj));
            }
            Value::Object(o)
        })
        .collect();
    body["mcp_servers"] = json!(mapped);
}

/// `providerOptions.anthropic.container` — programmatic tool calling (string
/// id) or agent skills (object with id + skills). Skills require the code
/// execution beta headers; returns an error when a custom skill's provider
/// reference lacks the `anthropic` key.
fn append_anthropic_container(
    body: &mut Value,
    options: &CallOptions,
    profile: &RequestProfile,
    warnings: &mut Vec<Warning>,
) -> Result<(), AiMuxError> {
    let Some(container) = anthropic_option(&options.provider_options, profile, "container") else {
        return Ok(());
    };
    let skills = container.get("skills").and_then(|s| s.as_array());
    if let Some(skills_arr) = skills.filter(|a| !a.is_empty()) {
        let mut container_obj = Map::new();
        if let Some(id) = container.get("id")
            && !id.is_null()
        {
            container_obj.insert("id".to_string(), id.clone());
        }
        let mut skills_mapped: Vec<Value> = Vec::new();
        for skill in skills_arr {
            let stype = skill.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let skill_id = if stype == "custom" {
                match skill
                    .get("providerReference")
                    .and_then(|r| r.get(CANONICAL))
                {
                    Some(id) => id.clone(),
                    None => {
                        return Err(AiMuxError::UnsupportedFunctionality(format!(
                            "skill provider reference is missing the '{CANONICAL}' key: {skill}"
                        )));
                    }
                }
            } else {
                skill.get("skillId").cloned().unwrap_or(Value::Null)
            };
            let mut s = Map::new();
            s.insert("type".to_string(), json!(stype));
            s.insert("skill_id".to_string(), skill_id);
            if let Some(v) = skill.get("version")
                && !v.is_null()
            {
                s.insert("version".to_string(), v.clone());
            }
            skills_mapped.push(Value::Object(s));
        }
        container_obj.insert("skills".to_string(), json!(skills_mapped));
        body["container"] = json!(container_obj);

        // Warn when skills are configured without a code execution tool.
        let has_code_exec = options
            .tools
            .as_ref()
            .map(|t| {
                t.iter().any(|tool| match tool {
                    Tool::Provider(pt) => {
                        pt.id == "anthropic.code_execution_20250825"
                            || pt.id == "anthropic.code_execution_20260120"
                    }
                    _ => false,
                })
            })
            .unwrap_or(false);
        if !has_code_exec {
            warnings.push(Warning::Other {
                message: "code execution tool is required when using skills".to_string(),
            });
        }
    } else if let Some(id) = container.get("id")
        && !id.is_null()
    {
        body["container"] = id.clone();
    }
    Ok(())
}

/// Build the Anthropic request body, returning warnings alongside the body.
///
/// Implements the thinking/reasoning pipeline from the TS
/// `anthropic-language-model.ts` `getArgs`: model-capability detection,
/// `rejectsSamplingParameters` stripping, top-level `reasoning` → thinking/
/// effort mapping (provider options take precedence), and thinking-enabled
/// default budget. The single oversized function was split into focused
/// helpers (issue M11); behavior is unchanged.
///
/// # Errors
///
/// Propagates prompt/provider-option conversion errors, e.g.
/// `AiMuxError::invalid_argument` for an unresolvable file reference or
/// container option.
#[cfg(test)]
pub(super) fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    build_request_body_for(model_id, options, stream, &RequestProfile::default())
}

/// [`build_request_body_with_warnings`] for a host of the Messages API other
/// than the first-party endpoint under its canonical name.
///
/// # Errors
///
/// Same as [`build_request_body_with_warnings`].
pub(crate) fn build_request_body_for(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
    profile: &RequestProfile,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut parsed_options = options.clone();
    if let Some(raw) =
        anthropic_options_in(options.provider_options.as_ref(), &profile.options_name)
    {
        let parsed = parse_anthropic_option_object("options", &Value::Object(raw))
            .map_err(|()| AiMuxError::invalid_argument("invalid anthropic provider options"))?;
        let namespaces = parsed_options
            .provider_options
            .get_or_insert_with(Default::default);
        namespaces.shift_remove(CANONICAL);
        namespaces.insert(
            profile.options_name.clone(),
            parsed.as_object().unwrap().clone(),
        );
    }
    let options = &parsed_options;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut betas: IndexSet<String> = IndexSet::new();
    let caps = get_model_capabilities(model_id);

    // Unknown-model max output tokens warning (TS L305-314): for models we do
    // not recognise, note the applied default limit when the caller did not
    // set maxOutputTokens.
    if !caps.is_known_model && options.max_output_tokens.is_none() {
        warnings.push(Warning::Compatibility {
            feature: "maxOutputTokens".to_string(),
            details: Some(format!(
                "The model \"{}\" is unknown. The max output tokens have been limited to {}. Set maxOutputTokens explicitly to override this limit.",
                model_id, caps.max_output_tokens
            )),
        });
    }

    // rejectsSamplingParameters: strip temperature/topK/topP with warnings.
    let (mut temperature, mut top_p, mut top_k) =
        strip_anthropic_sampling_params(options, model_id, &caps, &mut warnings);

    // providerOptions.anthropic.thinking / .effort + top-level `reasoning`.
    let (thinking_config, thinking_effort) =
        resolve_anthropic_thinking(model_id, options, profile, &caps, &mut warnings);
    let (thinking_type, send_thinking, mut thinking_budget, thinking_display) =
        derive_anthropic_thinking(&thinking_config);

    let max_tokens = options.max_output_tokens.unwrap_or(caps.max_output_tokens);

    let send_input_reasoning =
        anthropic_option(&options.provider_options, profile, "sendReasoning")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
    let mut cache_validator = CacheControlValidator::for_options_name(&profile.options_name);
    let conversion = convert_prompt_with_validator(
        &options.prompt,
        send_input_reasoning,
        &ToolNameMapping::new(options.tools.as_deref()),
        &profile.options_name,
        &mut cache_validator,
    )?;
    let system = conversion.system;
    let messages = conversion.messages;
    betas.extend(conversion.betas);
    warnings.extend(conversion.warnings);

    let mut body = json!({
        "model": model_id,
        "messages": messages,
        "max_tokens": max_tokens,
    });
    // `stream` is sent only when streaming, as the AI SDK does.
    if stream {
        body["stream"] = json!(true);
    }

    if let Some(sys) = system {
        body["system"] = json!(sys);
    }

    insert_anthropic_thinking(
        &mut body,
        &thinking_type,
        send_thinking,
        &thinking_budget,
        &thinking_display,
        &thinking_effort,
    );

    if let Some(binding) = thinking_config.as_ref().and_then(|v| v.get("blockBinding")) {
        if body.get("thinking").is_none() {
            body["thinking"] = json!({});
        }
        body["thinking"]["block_binding"] =
            json!({ "prefix_mismatch_behavior": binding.get("prefixMismatchBehavior") });
    }

    // Thinking-enabled post-processing (TS L651-696): default budget,
    // sampling-parameter stripping, `max_tokens` adjustment.
    let mut adjusted_max_tokens = apply_anthropic_thinking_post_processing(
        &mut body,
        &thinking_type,
        &mut thinking_budget,
        max_tokens,
        &mut temperature,
        &mut top_k,
        &mut top_p,
        &mut warnings,
    );
    if caps.is_known_model && adjusted_max_tokens > caps.max_output_tokens {
        if options.max_output_tokens.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "maxOutputTokens".to_string(),
                details: Some(format!(
                    "{} (maxOutputTokens + thinkingBudget) is greater than {} {} max output tokens. The max output tokens have been limited to {}.",
                    adjusted_max_tokens, model_id, caps.max_output_tokens, caps.max_output_tokens
                )),
            });
        }
        adjusted_max_tokens = caps.max_output_tokens;
    }
    if adjusted_max_tokens != max_tokens {
        body["max_tokens"] = json!(adjusted_max_tokens);
    }

    if !matches!(
        thinking_type.as_deref(),
        Some("enabled" | "adaptive" | "between_tools")
    ) && (caps.is_known_model || is_legacy_claude(model_id))
        && temperature.is_some()
        && top_p.is_some()
    {
        warnings.push(Warning::Unsupported {
            feature: "topP".to_string(),
            details: Some(
                "topP is not supported when temperature is set. topP is ignored.".to_string(),
            ),
        });
        top_p = None;
    }
    insert_anthropic_sampling(&mut body, temperature, top_p, top_k, options);

    let mut json_tool_schema = None;
    if let Some(ResponseFormat::Json { schema, .. }) = &options.response_format {
        if let Some(schema) = schema {
            let mode = anthropic_option(&options.provider_options, profile, "structuredOutputMode");
            let mut use_native = mode.as_ref().and_then(Value::as_str) == Some("outputFormat")
                || (mode.as_ref().and_then(Value::as_str).unwrap_or("auto") == "auto"
                    && caps.supports_structured_output
                    && profile.supports_native_structured_output);
            if !use_native
                && caps.rejects_forced_tool_use
                && caps.supports_structured_output
                && profile.supports_native_structured_output
            {
                warnings.push(Warning::Unsupported {
                    feature: "providerOptions.anthropic.structuredOutputMode".to_string(),
                    details: Some(format!("structuredOutputMode 'jsonTool' is not supported by {model_id} because it rejects forced tool use. Using 'outputFormat' instead.")),
                });
                use_native = true;
            }
            if !use_native {
                json_tool_schema = Some(schema);
                if anthropic_option(&options.provider_options, profile, "disableParallelToolUse")
                    .and_then(|v| v.as_bool())
                    == Some(false)
                {
                    warnings.push(Warning::Unsupported {
                        feature: "providerOptions.anthropic.disableParallelToolUse".to_string(),
                        details: Some("`disableParallelToolUse: false` is ignored when using the JSON response tool. Parallel tool use is disabled to ensure a single coherent JSON tool call.".to_string()),
                    });
                }
            }
            if use_native {
                if body.get("output_config").is_none() {
                    body["output_config"] = json!({});
                }
                body["output_config"]["format"] = json!({ "type": "json_schema", "schema": super::sanitize_json_schema::sanitize_json_schema(schema) });
            }
        } else {
            warnings.push(Warning::Unsupported {
                feature: "responseFormat".to_string(),
                details: Some(
                    "JSON response format requires a schema. The response format is ignored."
                        .to_string(),
                ),
            });
        }
    }

    // providerOptions.anthropic.metadata.userId -> metadata.user_id.
    if let Some(user_id) = anthropic_option(&options.provider_options, profile, "metadata")
        .and_then(|metadata| metadata.get("userId").cloned())
        .filter(|user_id| !user_id.is_null())
    {
        body["metadata"] = json!({ "user_id": user_id });
    }

    // Tools — provider-defined tools alongside function tools, plus the
    // required beta headers / tool warnings.
    let mut tool_betas = IndexSet::new();
    apply_anthropic_tools(
        &mut body,
        options,
        profile,
        stream,
        json_tool_schema,
        &caps,
        &mut tool_betas,
        &mut warnings,
        &mut cache_validator,
    )?;
    warnings.extend(cache_validator.take_warnings());

    // providerOptions.anthropic.mcpServers → mcp_servers + beta header.
    append_anthropic_mcp_servers(&mut body, options, profile);

    // providerOptions.anthropic.container — programmatic tool calling / skills.
    append_anthropic_container(&mut body, options, profile, &mut warnings)?;

    let provider_options =
        anthropic_options_in(options.provider_options.as_ref(), &profile.options_name);
    if let Some(provider_options) = provider_options {
        if provider_options
            .get("compaction")
            .is_some_and(|v| !v.is_null())
            && provider_options
                .get("contextManagement")
                .is_some_and(|v| !v.is_null())
        {
            return Err(AiMuxError::invalid_argument(
                "Anthropic provider options `compaction` and `contextManagement` cannot be used together.",
            ));
        }
        for (source, target) in [
            ("speed", "speed"),
            ("serviceTier", "service_tier"),
            ("inferenceGeo", "inference_geo"),
            ("cacheControl", "cache_control"),
            ("compaction", "compaction"),
        ] {
            if let Some(value) = provider_options.get(source).filter(|v| !v.is_null()) {
                body[target] = value.clone();
            }
        }
        if let Some(value) = provider_options.get("fallbacks").filter(|v| {
            v.as_str() == Some("default") || v.as_array().is_some_and(|a| !a.is_empty())
        }) {
            body["fallbacks"] = value.clone();
        }
        if let Some(safeguards) = provider_options
            .get("safeguards")
            .and_then(Value::as_array)
            .filter(|safeguards| !safeguards.is_empty())
        {
            let mapped: Vec<Value> = safeguards
                .iter()
                .map(|safeguard| {
                    let mut mapped = json!({ "type": field(safeguard, "type") });
                    if let Some(context) = safeguard.get("classifierContext") {
                        mapped["classifier_context"] = context.clone();
                    }
                    mapped
                })
                .collect();
            body["safeguards"] = json!(mapped);
        }
        if let Some(value) = provider_options.get("taskBudget").filter(|v| !v.is_null()) {
            if body.get("output_config").is_none() {
                body["output_config"] = json!({});
            }
            body["output_config"]["task_budget"] = value.clone();
        }
        if let Some(edits) = provider_options
            .get("contextManagement")
            .and_then(|v| v.get("edits"))
            .and_then(Value::as_array)
        {
            let mut mapped = Vec::new();
            for edit in edits {
                let strategy = edit.get("type").and_then(Value::as_str).unwrap_or_default();
                let fields: &[(&str, &str)] = match strategy {
                    "clear_tool_uses_20250919" => &[
                        ("trigger", "trigger"),
                        ("keep", "keep"),
                        ("clearAtLeast", "clear_at_least"),
                        ("clearToolInputs", "clear_tool_inputs"),
                        ("excludeTools", "exclude_tools"),
                    ],
                    "clear_thinking_20251015" => &[("keep", "keep")],
                    "compact_20260112" => &[
                        ("trigger", "trigger"),
                        ("pauseAfterCompaction", "pause_after_compaction"),
                        ("instructions", "instructions"),
                    ],
                    _ => {
                        warnings.push(Warning::Other {
                            message: format!("Unknown context management strategy: {strategy}"),
                        });
                        continue;
                    }
                };
                let mut mapped_edit = json!({ "type": strategy });
                for (source, target) in fields {
                    if let Some(value) = edit.get(*source) {
                        mapped_edit[*target] = value.clone();
                    }
                }
                mapped.push(mapped_edit);
            }
            body["context_management"] = json!({ "edits": mapped });
        }
    }
    betas.extend(
        provider_option_betas(&body, &thinking_display)
            .into_iter()
            .map(str::to_string),
    );
    betas.extend(tool_betas);
    let option_betas = anthropic_option(&options.provider_options, profile, "anthropicBeta")
        .and_then(|value| value.as_array().cloned())
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Ok(RequestBodyResult {
        uses_json_response_tool: json_tool_schema.is_some(),
        body,
        warnings,
        betas,
        option_betas,
    })
}

/// The betas the provider options in `body` need, in the order
/// `anthropic-language-model.ts` adds them (L963-1037).
fn provider_option_betas(body: &Value, thinking_display: &Option<Value>) -> Vec<&'static str> {
    let has = |key: &str| body.get(key).is_some_and(|value| !value.is_null());
    let compact_edit = body
        .pointer("/context_management/edits")
        .and_then(Value::as_array)
        .is_some_and(|edits| edits.iter().any(|edit| edit["type"] == "compact_20260112"));
    let skills = body.get("container").is_some_and(Value::is_object);
    let fallbacks = body
        .get("fallbacks")
        .map(|value| value.as_str() == Some("default"));
    [
        (has("mcp_servers"), "mcp-client-2025-04-04"),
        (has("safeguards"), "dangerous-tool-use-2026-09-03"),
        (has("compaction"), "compact-2026-09-04"),
        (has("context_management"), "context-management-2025-06-27"),
        (compact_edit, "compact-2026-01-12"),
        (skills, "code-execution-2025-08-25"),
        (skills, "skills-2025-10-02"),
        (skills, "files-api-2025-04-14"),
        (
            body.pointer("/output_config/task_budget").is_some(),
            "task-budgets-2026-03-13",
        ),
        (body["speed"] == "fast", "fast-mode-2026-02-01"),
        (
            thinking_display.as_ref().and_then(Value::as_str) == Some("updates"),
            "thinking-display-updates-2026-08-18",
        ),
        (
            body.pointer("/thinking/block_binding").is_some(),
            "thinking-binding-controls-2026-08-01",
        ),
        (fallbacks == Some(true), "server-side-fallback-2026-07-01"),
        (fallbacks == Some(false), "server-side-fallback-2026-06-01"),
    ]
    .into_iter()
    .filter_map(|(needed, beta)| needed.then_some(beta))
    .collect()
}
