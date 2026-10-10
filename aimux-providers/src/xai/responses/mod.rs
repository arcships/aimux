//! xAI Responses API language model.
//!
//! Implements `LanguageModel` against xAI's `/responses` endpoint.
//! Mirrors the TS `XaiResponsesLanguageModel`.
//!
//! Key differences from the chat model:
//! - Uses `input` (not `messages`) with Responses API format
//! - `reasoning: { effort, summary }` object (not `reasoning_effort` string)
//! - `text.format` for structured output (not `response_format`)
//! - Provider-executed tools (web_search, x_search, code_execution, etc.)
//! - ~65 streaming event types (response.created, response.output_text.delta, etc.)
//! - `cost_in_usd_ticks` in providerMetadata
//! - Citations via annotations → sources

pub mod convert;
pub mod types;

use aimux_core::tool::RawToolCall;
use aimux_core::tool::ToolResult;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use aimux_provider_utils::{TransformStreamController, Transformer, pipe_through};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use crate::shared::EndpointConfig;
use convert::{
    build_responses_request_body, convert_xai_responses_usage, get_tool_input,
    map_xai_responses_finish_reason, resolve_tool_name,
};

/// Global counter for generating unique source IDs.
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_source_id() -> String {
    format!("id-{}", SOURCE_ID_COUNTER.fetch_add(1, Ordering::SeqCst))
}

fn zero_usage() -> Usage {
    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(0),
            no_cache: Some(0),
            cache_read: Some(0),
            cache_write: Some(0),
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(0),
            text: Some(0),
            reasoning: Some(0),
        },
        raw: None,
    }
}

fn response_provider_metadata(
    cost: Option<u64>,
    service_tier: Option<&str>,
    prompt_cache_key: Option<&str>,
    safety_identifier: Option<&str>,
) -> Option<aimux_core::types::ProviderMetadata> {
    let mut metadata = serde_json::Map::new();
    if let Some(cost) = cost {
        metadata.insert("costInUsdTicks".into(), json!(cost));
    }
    for (key, value) in [
        ("serviceTier", service_tier),
        ("promptCacheKey", prompt_cache_key),
        ("safetyIdentifier", safety_identifier),
    ] {
        if let Some(value) = value {
            metadata.insert(key.into(), json!(value));
        }
    }
    (!metadata.is_empty()).then(|| crate::xai::options::xai_metadata(Value::Object(metadata)))
}

fn map_web_search_action(action: &Value) -> Value {
    let Some(kind) = action.get("type").and_then(Value::as_str) else {
        return json!({});
    };
    let fields: &[&str] = match kind {
        "search" => &["query"],
        "open_page" => &["url"],
        "find_in_page" => &["url", "pattern"],
        _ => return json!({}),
    };
    if fields.iter().any(|key| {
        action
            .get(key)
            .is_some_and(|v| !v.is_null() && !v.is_string())
    }) || action
        .get("sources")
        .is_some_and(|v| !v.is_null() && !v.is_array())
        || (kind == "search"
            && action.get("queries").is_some_and(|v| {
                !v.is_null() && !v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
            }))
    {
        return json!({});
    }
    let mapped_type = match kind {
        "open_page" => "openPage",
        "find_in_page" => "findInPage",
        _ => kind,
    };
    let mut mapped_action = json!({ "type": mapped_type });
    for key in fields {
        if let Some(value) = action.get(key)
            && (kind != "search" || !value.is_null())
        {
            mapped_action[key] = value.clone();
        }
    }
    if kind == "search"
        && let Some(queries) = action.get("queries").filter(|v| !v.is_null())
    {
        mapped_action["queries"] = queries.clone();
    }
    let mut result = json!({ "action": mapped_action });
    if let Some(sources) = action.get("sources").and_then(Value::as_array) {
        let sources: Vec<Value> = sources
            .iter()
            .filter(|source| source.get("type").and_then(Value::as_str) == Some("url"))
            .filter_map(|source| source.get("url").and_then(Value::as_str))
            .map(|url| json!({ "type": "url", "url": url }))
            .collect();
        if !sources.is_empty() {
            result["sources"] = json!(sources);
        }
    }
    result
}

fn image_generation_result(part: &Value, tool_name: String) -> ToolResult {
    let (result, is_error) = match part.get("result").filter(|value| !value.is_null()) {
        Some(result) => {
            let mut output = json!({ "result": result });
            if let Some(prompt) = part.get("prompt").filter(|value| !value.is_null()) {
                output["prompt"] = prompt.clone();
            }
            (output, None)
        }
        None => (
            json!(format!(
                "Image generation failed (status: {}).",
                part.get("status").and_then(Value::as_str).unwrap_or("")
            )),
            Some(true),
        ),
    };
    ToolResult {
        tool_call_id: part
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tool_name,
        result,
        is_error,
        preliminary: None,
        dynamic: None,
        provider_metadata: None,
    }
}

/// An xAI Responses language model (Grok).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct XaiResponsesModel {
    model_id: String,
    config: EndpointConfig,
}

impl XaiResponsesModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

#[async_trait]
impl LanguageModel for XaiResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let request_result = build_responses_request_body(&self.model_id, options, false)?;
        let body = request_result.body;
        let provider_tool_names = request_result.provider_tool_names;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/responses"), options),
            body.clone(),
            super::xai_successful_response_handler::<types::XaiResponsesResponse>(),
            super::xai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_value = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let mut content: Vec<GenerateContent> = Vec::new();
        let mut has_function_call = false;

        for part in &data.output {
            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if part_type == "image_generation_call" {
                let tool_name = resolve_tool_name(part_type, None, &provider_tool_names);
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    tool_name: tool_name.clone(),
                    input: "{}".to_string(),
                    provider_executed: Some(true),
                    dynamic: None,
                    provider_metadata: None,
                }));
                content.push(GenerateContent::ToolResult(image_generation_result(
                    part, tool_name,
                )));
                continue;
            }

            // ── file_search_call ──
            if part_type == "file_search_call" {
                let tool_name = resolve_tool_name(part_type, None, &provider_tool_names);
                let part_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");

                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: part_id.to_string(),
                    tool_name: tool_name.clone(),
                    input: String::new(),
                    provider_executed: Some(true),
                    dynamic: None,
                    provider_metadata: None,
                }));

                let queries = part
                    .get("queries")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let results =
                    part.get("results")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            json!(arr.iter().map(|r| json!({
                        "fileId": r.get("file_id").cloned().unwrap_or(Value::Null),
                        "filename": r.get("filename").cloned().unwrap_or(Value::Null),
                        "score": r.get("score").cloned().unwrap_or(Value::Null),
                        "text": r.get("text").cloned().unwrap_or(Value::Null),
                    })).collect::<Vec<_>>())
                        })
                        .unwrap_or(Value::Null);

                content.push(GenerateContent::ToolResult(ToolResult {
                    tool_call_id: part_id.to_string(),
                    tool_name,
                    result: json!({ "queries": queries, "results": results }),
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
                continue;
            }

            // ── Server-side tool calls ──
            if matches!(
                part_type,
                "web_search_call"
                    | "x_search_call"
                    | "code_interpreter_call"
                    | "code_execution_call"
                    | "view_image_call"
                    | "view_x_video_call"
                    | "custom_tool_call"
                    | "mcp_call"
            ) {
                let part_name = part.get("name").and_then(|v| v.as_str());
                let tool_name = resolve_tool_name(part_type, part_name, &provider_tool_names);
                let tool_input = get_tool_input(part_type, part);
                let part_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");

                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: part_id.to_string(),
                    tool_name: tool_name.clone(),
                    input: tool_input,
                    provider_executed: Some(true),
                    dynamic: None,
                    provider_metadata: None,
                }));
                if part_type == "web_search_call" {
                    content.push(GenerateContent::ToolResult(ToolResult {
                        tool_call_id: part_id.to_string(),
                        tool_name,
                        result: map_web_search_action(part.get("action").unwrap_or(&Value::Null)),
                        is_error: None,
                        preliminary: None,
                        dynamic: None,
                        provider_metadata: None,
                    }));
                }
                continue;
            }

            match part_type {
                "message" => {
                    let content_parts = part
                        .get("content")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for cp in &content_parts {
                        if let Some(text) = cp.get("text").and_then(|v| v.as_str())
                            && !text.is_empty()
                        {
                            content.push(GenerateContent::Text {
                                text: text.to_string(),
                                provider_metadata: None,
                            });
                        }
                        if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array())
                        {
                            for ann in annotations {
                                if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation")
                                {
                                    let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                    let title = ann
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or(url)
                                        .to_string();
                                    content.push(GenerateContent::Source(Source::Url {
                                        id: generate_source_id(),
                                        url: url.to_string(),
                                        title: Some(title),
                                        provider_metadata: None,
                                    }));
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    has_function_call = true;
                    let call_id = part.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                    let name = part.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let arguments = part.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                    let input = arguments.to_string();
                    content.push(GenerateContent::ToolCall(RawToolCall {
                        tool_call_id: call_id.to_string(),
                        tool_name: name.to_string(),
                        input,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: None,
                    }));
                }
                "reasoning" => {
                    let summary = part.get("summary").and_then(|v| v.as_array());
                    let reasoning_content = part.get("content").and_then(|v| v.as_array());

                    let texts: Vec<String> = if let Some(summary) = summary {
                        if !summary.is_empty() {
                            summary
                                .iter()
                                .filter_map(|s| {
                                    s.get("text")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string)
                                })
                                .filter(|t| !t.is_empty())
                                .collect()
                        } else {
                            reasoning_content
                                .map(|c| {
                                    c.iter()
                                        .filter_map(|s| {
                                            s.get("text")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string)
                                        })
                                        .filter(|t| !t.is_empty())
                                        .collect()
                                })
                                .unwrap_or_default()
                        }
                    } else {
                        Vec::new()
                    };

                    let reasoning_text = texts.join("");
                    let encrypted_content = part
                        .get("encrypted_content")
                        .and_then(|v| v.as_str())
                        .filter(|v| !v.is_empty());
                    let item_id = part
                        .get("id")
                        .and_then(|v| v.as_str())
                        .filter(|v| !v.is_empty());

                    if !reasoning_text.is_empty() || encrypted_content.is_some() {
                        let mut meta = json!({});
                        if let Some(ec) = encrypted_content {
                            meta["reasoningEncryptedContent"] = json!(ec);
                        }
                        if let Some(id) = item_id {
                            meta["itemId"] = json!(id);
                        }
                        content.push(GenerateContent::Reasoning(ReasoningOutput {
                            text: reasoning_text,
                            provider_metadata: (encrypted_content.is_some() || item_id.is_some())
                                .then(|| crate::xai::options::xai_metadata(meta)),
                        }));
                    }
                }
                _ => {}
            }
        }

        let finish_reason = FinishReason {
            unified: if has_function_call {
                FinishReasonUnified::ToolCalls
            } else {
                data.status
                    .as_deref()
                    .map(map_xai_responses_finish_reason)
                    .unwrap_or(FinishReasonUnified::Other)
            },
            raw: data.status.clone(),
        };

        let usage = data.usage.as_ref().map_or_else(zero_usage, |u| {
            let mut usage = convert_xai_responses_usage(u);
            usage.raw = raw_value
                .get("usage")
                .and_then(|value| value.as_object().cloned());
            usage
        });
        let provider_metadata = response_provider_metadata(
            data.usage.as_ref().and_then(|u| u.cost_in_usd_ticks),
            data.service_tier.as_deref(),
            data.prompt_cache_key.as_deref(),
            data.safety_identifier.as_deref(),
        );

        let timestamp = data
            .created_at
            .and_then(|c| chrono::DateTime::from_timestamp(c as i64, 0).map(|dt| dt.to_rfc3339()));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_result.warnings,
            provider_metadata,
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.id,
                timestamp,
                model_id: data.model,
                headers: Some(response_headers),
                body: Some(raw_value),
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let request_result = build_responses_request_body(&self.model_id, options, true)?;
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_tool_names = request_result.provider_tool_names;
        let endpoint = exchange.url("/responses");

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            super::xai_event_source_response_handler::<Value>(),
            super::xai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first => first,
        };
        if let Some(Ok(event)) = &first_event
            && types::event_type(event) == "error"
        {
            return Err(super::xai_stream_error(
                event,
                &endpoint,
                body.clone(),
                response_headers.clone(),
            ));
        }
        let stream_error_url = endpoint;
        let stream_request_body = body.clone();
        let stream_response_headers = response_headers.clone();
        let include_raw_chunks = options.include_raw_chunks == Some(true);

        let stream = pipe_through(
            futures::stream::iter(first_event).chain(sse_stream),
            XaiResponsesStream {
                warnings,
                include_raw_chunks,
                provider_tool_names,
                stream_error_url,
                stream_request_body,
                stream_response_headers,
                final_usage: None,
                final_finish_reason: FinishReason {
                    unified: FinishReasonUnified::Other,
                    raw: None,
                },
                cost_in_usd_ticks: None,
                service_tier: None,
                prompt_cache_key: None,
                safety_identifier: None,
                has_function_call: false,
                is_first_chunk: true,
                content_blocks: Vec::new(),
                seen_tool_calls: std::collections::HashSet::new(),
                active_reasoning: HashMap::new(),
                ongoing_tool_calls: HashMap::new(),
                finished: false,
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
}

/// The `TransformStream` of the xAI Responses `doStream`.
struct XaiResponsesStream {
    warnings: Vec<aimux_core::types::Warning>,
    include_raw_chunks: bool,
    provider_tool_names: HashMap<String, String>,
    stream_error_url: String,
    stream_request_body: Value,
    stream_response_headers: HashMap<String, String>,
    final_usage: Option<Usage>,
    final_finish_reason: FinishReason,
    cost_in_usd_ticks: Option<u64>,
    service_tier: Option<String>,
    prompt_cache_key: Option<String>,
    safety_identifier: Option<String>,
    has_function_call: bool,
    is_first_chunk: bool,
    /// Open text blocks.
    content_blocks: Vec<String>,
    seen_tool_calls: std::collections::HashSet<String>,
    active_reasoning: HashMap<String, ()>,
    /// Ongoing function calls: output_index -> (tool_call_id, tool_name).
    ongoing_tool_calls: HashMap<u64, (String, String)>,
    /// Set by an `error` event: later chunks are ignored, the stream still
    /// finishes.
    finished: bool,
}

impl Transformer for XaiResponsesStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut TransformStreamController<StreamPart>) {
        controller.enqueue(StreamPart::StreamStart {
            warnings: std::mem::take(&mut self.warnings),
        });
    }

    fn transform(
        &mut self,
        event: Result<Value, AiMuxError>,
        controller: &mut TransformStreamController<StreamPart>,
    ) {
        if self.finished {
            return;
        }
        let Self {
            include_raw_chunks,
            ref provider_tool_names,
            ref stream_error_url,
            ref stream_request_body,
            ref stream_response_headers,
            ref mut final_usage,
            ref mut final_finish_reason,
            ref mut cost_in_usd_ticks,
            ref mut service_tier,
            ref mut prompt_cache_key,
            ref mut safety_identifier,
            ref mut has_function_call,
            ref mut is_first_chunk,
            ref mut content_blocks,
            ref mut seen_tool_calls,
            ref mut active_reasoning,
            ref mut ongoing_tool_calls,
            ref mut finished,
            ..
        } = *self;
        match event {
            Ok(parsed) => {
                if include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: parsed.clone(),
                    });
                }
                let event_type = types::event_type(&parsed);

                // ── response.created / response.in_progress ──
                if event_type == "response.created" || event_type == "response.in_progress" {
                    if *is_first_chunk {
                        *is_first_chunk = false;
                        let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                        let id = response
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string);
                        let model = response
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(std::string::ToString::to_string);
                        let created_at = response
                            .get("created_at")
                            .and_then(serde_json::Value::as_u64);
                        let timestamp = created_at.and_then(|c| {
                            chrono::DateTime::from_timestamp(c as i64, 0).map(|dt| dt.to_rfc3339())
                        });
                        controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                            id,
                            timestamp,
                            model_id: model,
                        }));
                    }
                    return;
                }

                // ── reasoning_summary_part.added ──
                if event_type == "response.reasoning_summary_part.added" {
                    let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                    let block_id = format!("reasoning-{item_id}");
                    if !active_reasoning.contains_key(item_id) {
                        active_reasoning.insert(item_id.to_string(), ());
                        controller.enqueue(StreamPart::ReasoningStart {
                            id: block_id,
                            provider_metadata: Some(crate::xai::options::xai_metadata(
                                json!({ "itemId": item_id }),
                            )),
                        });
                    }
                    return;
                }

                // ── reasoning_summary_text.delta ──
                if event_type == "response.reasoning_summary_text.delta" {
                    let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                    let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                    let block_id = format!("reasoning-{item_id}");
                    controller.enqueue(StreamPart::ReasoningDelta {
                        id: block_id,
                        delta: delta.to_string(),
                        provider_metadata: Some(crate::xai::options::xai_metadata(
                            json!({ "itemId": item_id }),
                        )),
                    });
                    return;
                }

                // ── reasoning_summary_text.done ──
                if event_type == "response.reasoning_summary_text.done" {
                    return;
                }

                // ── reasoning_text.delta ──
                if event_type == "response.reasoning_text.delta" {
                    let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                    let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                    let block_id = format!("reasoning-{item_id}");
                    if !active_reasoning.contains_key(item_id) {
                        active_reasoning.insert(item_id.to_string(), ());
                        controller.enqueue(StreamPart::ReasoningStart {
                            id: block_id.clone(),
                            provider_metadata: Some(crate::xai::options::xai_metadata(
                                json!({ "itemId": item_id }),
                            )),
                        });
                    }
                    controller.enqueue(StreamPart::ReasoningDelta {
                        id: block_id,
                        delta: delta.to_string(),
                        provider_metadata: Some(crate::xai::options::xai_metadata(
                            json!({ "itemId": item_id }),
                        )),
                    });
                    return;
                }

                // ── reasoning_text.done ──
                if event_type == "response.reasoning_text.done" {
                    return;
                }

                // ── output_text.delta ──
                if event_type == "response.output_text.delta" {
                    let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                    let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                    let block_id = format!("text-{item_id}");
                    if !content_blocks.contains(&block_id) {
                        content_blocks.push(block_id.clone());
                        controller.enqueue(StreamPart::TextStart {
                            id: block_id.clone(),
                            provider_metadata: None,
                        });
                    }
                    controller.enqueue(StreamPart::TextDelta {
                        id: block_id,
                        delta: delta.to_string(),
                        provider_metadata: None,
                    });
                    return;
                }

                // ── output_text.done ──
                if event_type == "response.output_text.done" {
                    if let Some(annotations) = parsed.get("annotations").and_then(|v| v.as_array())
                    {
                        for ann in annotations {
                            if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                let title = ann
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(url)
                                    .to_string();
                                controller.enqueue(StreamPart::Source(Source::Url {
                                    id: generate_source_id(),
                                    url: url.to_string(),
                                    title: Some(title),
                                    provider_metadata: None,
                                }));
                            }
                        }
                    }
                    return;
                }

                // ── output_text.annotation.added ──
                if event_type == "response.output_text.annotation.added" {
                    let annotation = parsed.get("annotation").cloned().unwrap_or(Value::Null);
                    if annotation.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                        let url = annotation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                        let title = annotation
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or(url)
                            .to_string();
                        controller.enqueue(StreamPart::Source(Source::Url {
                            id: generate_source_id(),
                            url: url.to_string(),
                            title: Some(title),
                            provider_metadata: None,
                        }));
                    }
                    return;
                }

                // ── response.done / response.completed / response.incomplete ──
                if event_type == "response.done"
                    || event_type == "response.completed"
                    || event_type == "response.incomplete"
                {
                    let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                    if let Some(usage) = response.get("usage")
                        && let Ok(u) =
                            serde_json::from_value::<types::XaiResponsesUsage>(usage.clone())
                    {
                        let mut converted = convert_xai_responses_usage(&u);
                        converted.raw = usage.as_object().cloned();
                        *final_usage = Some(converted);
                        *cost_in_usd_ticks = u.cost_in_usd_ticks;
                    }

                    *service_tier = response
                        .get("service_tier")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    *prompt_cache_key = response
                        .get("prompt_cache_key")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    *safety_identifier = response
                        .get("safety_identifier")
                        .and_then(Value::as_str)
                        .map(str::to_string);

                    if event_type == "response.incomplete" {
                        let reason = response
                            .get("incomplete_details")
                            .and_then(|d| d.get("reason"))
                            .and_then(|v| v.as_str());
                        *final_finish_reason = FinishReason {
                            unified: reason
                                .map(map_xai_responses_finish_reason)
                                .unwrap_or(FinishReasonUnified::Other),
                            raw: reason
                                .map(std::string::ToString::to_string)
                                .or(Some("incomplete".to_string())),
                        };
                    } else if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
                        *final_finish_reason = FinishReason {
                            unified: if *has_function_call {
                                FinishReasonUnified::ToolCalls
                            } else {
                                map_xai_responses_finish_reason(status)
                            },
                            raw: Some(status.to_string()),
                        };
                    }
                    return;
                }

                // ── response.failed ──
                if event_type == "response.failed" {
                    let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                    let reason = response
                        .get("incomplete_details")
                        .and_then(|d| d.get("reason"))
                        .and_then(|v| v.as_str());
                    *final_finish_reason = FinishReason {
                        unified: reason
                            .map(map_xai_responses_finish_reason)
                            .unwrap_or(FinishReasonUnified::Error),
                        raw: reason
                            .map(std::string::ToString::to_string)
                            .or(Some("error".to_string())),
                    };
                    if let Some(usage) = response.get("usage")
                        && let Ok(u) =
                            serde_json::from_value::<types::XaiResponsesUsage>(usage.clone())
                    {
                        let mut converted = convert_xai_responses_usage(&u);
                        converted.raw = usage.as_object().cloned();
                        *final_usage = Some(converted);
                    }
                    if response.get("error").is_some_and(|error| !error.is_null()) {
                        controller.enqueue(StreamPart::Error {
                            error: super::xai_stream_error(
                                &parsed,
                                stream_error_url,
                                stream_request_body.clone(),
                                stream_response_headers.clone(),
                            ),
                        });
                    }
                    return;
                }

                // ── error event ──
                if event_type == "error" {
                    controller.enqueue(StreamPart::Error {
                        error: super::xai_stream_error(
                            &parsed,
                            stream_error_url,
                            stream_request_body.clone(),
                            stream_response_headers.clone(),
                        ),
                    });
                    *final_finish_reason = FinishReason {
                        unified: FinishReasonUnified::Error,
                        raw: Some("error".to_string()),
                    };
                    *finished = true;
                    return;
                }

                // ── custom_tool_call_input.delta / .done ──
                if event_type == "response.custom_tool_call_input.delta"
                    || event_type == "response.custom_tool_call_input.done"
                {
                    return;
                }

                // ── function_call_arguments.delta ──
                if event_type == "response.function_call_arguments.delta" {
                    let output_index = parsed
                        .get("output_index")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some((tool_call_id, _)) = ongoing_tool_calls.get(&output_index) {
                        controller.enqueue(StreamPart::ToolInputDelta {
                            id: tool_call_id.clone(),
                            delta: delta.to_string(),
                            provider_metadata: None,
                        });
                    }
                    return;
                }

                // ── function_call_arguments.done ──
                if event_type == "response.function_call_arguments.done" {
                    return;
                }

                // ── output_item.added / output_item.done ──
                if event_type == "response.output_item.added"
                    || event_type == "response.output_item.done"
                    || matches!(
                        event_type,
                        "response.image_generation_call.in_progress"
                            | "response.image_generation_call.generating"
                            | "response.image_generation_call.completed"
                    )
                {
                    let item = if event_type.starts_with("response.image_generation_call.") {
                        json!({ "type": "image_generation_call", "id": parsed.get("item_id") })
                    } else {
                        parsed.get("item").cloned().unwrap_or(Value::Null)
                    };
                    let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let output_index = parsed
                        .get("output_index")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);

                    // ── reasoning item ──
                    if item_type == "reasoning" {
                        if event_type == "response.output_item.done" {
                            let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("reasoning-{part_id}");
                            let encrypted = item
                                .get("encrypted_content")
                                .and_then(|v| v.as_str())
                                .filter(|v| !v.is_empty());

                            if !active_reasoning.contains_key(part_id) {
                                active_reasoning.insert(part_id.to_string(), ());
                                controller.enqueue(StreamPart::ReasoningStart {
                                    id: block_id.clone(),
                                    provider_metadata: Some(crate::xai::options::xai_metadata(
                                        if part_id.is_empty() {
                                            json!({})
                                        } else {
                                            json!({ "itemId": part_id })
                                        },
                                    )),
                                });
                            }

                            let mut meta = json!({});
                            if !part_id.is_empty() {
                                meta["itemId"] = json!(part_id);
                            }
                            if let Some(ec) = encrypted {
                                meta["reasoningEncryptedContent"] = json!(ec);
                            }
                            controller.enqueue(StreamPart::ReasoningEnd {
                                id: block_id,
                                provider_metadata: Some(crate::xai::options::xai_metadata(json!(
                                    meta
                                ))),
                            });
                            active_reasoning.remove(part_id);
                        }
                        return;
                    }

                    // ── file_search_call item ──
                    if item_type == "file_search_call" || item_type == "image_generation_call" {
                        let tool_name = resolve_tool_name(item_type, None, provider_tool_names);
                        let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let input = if item_type == "image_generation_call" {
                            "{}"
                        } else {
                            ""
                        };

                        if !seen_tool_calls.contains(part_id) {
                            seen_tool_calls.insert(part_id.to_string());
                            controller.enqueue(StreamPart::ToolInputStart {
                                id: part_id.to_string(),
                                tool_name: tool_name.clone(),
                                provider_executed: None,
                                dynamic: None,
                                title: None,
                                provider_metadata: None,
                            });
                            controller.enqueue(StreamPart::ToolInputDelta {
                                id: part_id.to_string(),
                                delta: input.to_string(),
                                provider_metadata: None,
                            });
                            controller.enqueue(StreamPart::ToolInputEnd {
                                id: part_id.to_string(),
                                provider_metadata: None,
                            });
                            controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                tool_call_id: part_id.to_string(),
                                tool_name: tool_name.clone(),
                                input: input.to_string(),
                                provider_executed: Some(true),
                                dynamic: None,
                                provider_metadata: None,
                            }));
                        }

                        if event_type == "response.output_item.done" {
                            if item_type == "image_generation_call" {
                                controller.enqueue(StreamPart::ToolResult(
                                    image_generation_result(&item, tool_name),
                                ));
                                return;
                            }
                            let queries = item
                                .get("queries")
                                .and_then(|v| v.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let results = item.get("results").and_then(|v| v.as_array()).map(|arr| {
                                        json!(arr.iter().map(|r| json!({
                                            "fileId": r.get("file_id").cloned().unwrap_or(Value::Null),
                                            "filename": r.get("filename").cloned().unwrap_or(Value::Null),
                                            "score": r.get("score").cloned().unwrap_or(Value::Null),
                                            "text": r.get("text").cloned().unwrap_or(Value::Null),
                                        })).collect::<Vec<_>>())
                                    }).unwrap_or(Value::Null);
                            controller.enqueue(StreamPart::ToolResult(ToolResult {
                                tool_call_id: part_id.to_string(),
                                tool_name,
                                result: json!({ "queries": queries, "results": results }),
                                is_error: None,
                                preliminary: None,
                                dynamic: None,
                                provider_metadata: None,
                            }));
                        }
                        return;
                    }

                    // ── Server-side tool calls ──
                    if matches!(
                        item_type,
                        "web_search_call"
                            | "x_search_call"
                            | "code_interpreter_call"
                            | "code_execution_call"
                            | "view_image_call"
                            | "view_x_video_call"
                            | "custom_tool_call"
                            | "mcp_call"
                    ) {
                        let item_name = item.get("name").and_then(|v| v.as_str());
                        let tool_name =
                            resolve_tool_name(item_type, item_name, provider_tool_names);
                        let tool_input = get_tool_input(item_type, &item);
                        let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");

                        let should_emit = if item_type == "custom_tool_call" {
                            event_type == "response.output_item.done"
                        } else {
                            !seen_tool_calls.contains(part_id)
                        };

                        if should_emit && !seen_tool_calls.contains(part_id) {
                            seen_tool_calls.insert(part_id.to_string());
                            controller.enqueue(StreamPart::ToolInputStart {
                                id: part_id.to_string(),
                                tool_name: tool_name.clone(),
                                provider_executed: None,
                                dynamic: None,
                                title: None,
                                provider_metadata: None,
                            });
                            controller.enqueue(StreamPart::ToolInputDelta {
                                id: part_id.to_string(),
                                delta: tool_input.clone(),
                                provider_metadata: None,
                            });
                            controller.enqueue(StreamPart::ToolInputEnd {
                                id: part_id.to_string(),
                                provider_metadata: None,
                            });
                            controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                tool_call_id: part_id.to_string(),
                                tool_name: tool_name.clone(),
                                input: tool_input,
                                provider_executed: Some(true),
                                dynamic: None,
                                provider_metadata: None,
                            }));
                        }

                        if event_type == "response.output_item.done" {
                            controller.enqueue(StreamPart::ToolResult(ToolResult {
                                tool_call_id: part_id.to_string(),
                                tool_name,
                                result: if item_type == "web_search_call" {
                                    map_web_search_action(
                                        item.get("action").unwrap_or(&Value::Null),
                                    )
                                } else {
                                    json!({})
                                },
                                is_error: None,
                                preliminary: None,
                                dynamic: None,
                                provider_metadata: None,
                            }));
                        }
                        return;
                    }

                    // ── message item ──
                    if item_type == "message" {
                        let content_parts = item
                            .get("content")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        for cp in &content_parts {
                            if let Some(text) = cp.get("text").and_then(|v| v.as_str())
                                && !text.is_empty()
                            {
                                let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                let block_id = format!("text-{part_id}");
                                if !content_blocks.contains(&block_id) {
                                    content_blocks.push(block_id.clone());
                                    controller.enqueue(StreamPart::TextStart {
                                        id: block_id.clone(),
                                        provider_metadata: None,
                                    });
                                    controller.enqueue(StreamPart::TextDelta {
                                        id: block_id,
                                        delta: text.to_string(),
                                        provider_metadata: None,
                                    });
                                }
                            }
                            if let Some(annotations) =
                                cp.get("annotations").and_then(|v| v.as_array())
                            {
                                for ann in annotations {
                                    if ann.get("type").and_then(|v| v.as_str())
                                        == Some("url_citation")
                                    {
                                        let url =
                                            ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                        let title = ann
                                            .get("title")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or(url)
                                            .to_string();
                                        controller.enqueue(StreamPart::Source(Source::Url {
                                            id: generate_source_id(),
                                            url: url.to_string(),
                                            title: Some(title),
                                            provider_metadata: None,
                                        }));
                                    }
                                }
                            }
                        }
                        return;
                    }

                    // ── function_call item ──
                    if item_type == "function_call" {
                        let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");

                        if event_type == "response.output_item.added" {
                            ongoing_tool_calls
                                .insert(output_index, (call_id.to_string(), name.to_string()));
                            controller.enqueue(StreamPart::ToolInputStart {
                                id: call_id.to_string(),
                                tool_name: name.to_string(),
                                provider_executed: None,
                                dynamic: None,
                                title: None,
                                provider_metadata: None,
                            });
                        } else {
                            // output_item.done
                            *has_function_call = true;
                            ongoing_tool_calls.remove(&output_index);
                            let arguments =
                                item.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                            controller.enqueue(StreamPart::ToolInputEnd {
                                id: call_id.to_string(),
                                provider_metadata: None,
                            });
                            let input = arguments.to_string();
                            controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                tool_call_id: call_id.to_string(),
                                tool_name: name.to_string(),
                                input,
                                provider_executed: None,
                                dynamic: None,
                                provider_metadata: None,
                            }));
                        }
                    }
                }

                // All other event types (web_search_call.in_progress, etc.) are ignored.
            }
            Err(error) => {
                if error.is_recoverable_stream_error() {
                    if include_raw_chunks {
                        controller.enqueue(StreamPart::Raw {
                            raw_value: Value::Null,
                        });
                    }
                    controller.enqueue(StreamPart::Error { error });
                } else {
                    controller.error(error);
                }
            }
        }
    }

    fn flush(self, controller: &mut TransformStreamController<StreamPart>) {
        // Close any remaining open text blocks.
        for block_id in self.content_blocks {
            controller.enqueue(StreamPart::TextEnd {
                id: block_id,
                provider_metadata: None,
            });
        }

        // Final part: Finish.
        let provider_meta = response_provider_metadata(
            self.cost_in_usd_ticks,
            self.service_tier.as_deref(),
            self.prompt_cache_key.as_deref(),
            self.safety_identifier.as_deref(),
        );

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason,
            usage: self.final_usage.unwrap_or_else(zero_usage),
            provider_metadata: provider_meta,
        });
    }
}
