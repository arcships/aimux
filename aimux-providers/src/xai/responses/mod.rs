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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::{AiMuxError, ApiCallError};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send_stream_timed, send_timed};
use aimux_stream::SseStream;

use super::super::XAIConfig;
use crate::openai::responses::responses_convert::build_header_list;
use convert::{
    build_responses_request_body, convert_xai_responses_usage, get_tool_input,
    map_xai_responses_finish_reason, resolve_tool_name,
};

/// Global counter for generating unique source IDs.
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_source_id() -> String {
    format!("id-{}", SOURCE_ID_COUNTER.fetch_add(1, Ordering::SeqCst))
}

/// An xAI Responses language model (Grok).
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct XaiResponsesModel {
    model_id: String,
    config: XAIConfig,
}

impl XaiResponsesModel {
    pub fn new(model_id: String, config: XAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key()),
        );
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.base_url())
    }
}

#[async_trait]
impl LanguageModel for XaiResponsesModel {
    fn provider(&self) -> &str {
        "xai.responses"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        // M2b: reuse the OpenAI snapshot helper with xAI's provider name.
        crate::openai::config_snapshot_from_config(
            self.provider(),
            &self.model_id,
            self.config.openai_config(),
        )
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, false)?;
        let body = request_result.body;
        let provider_tool_names = request_result.provider_tool_names;

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            self.config.retry_config(),
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let raw_value: Value = serde_json::from_slice(&resp.body)?;

        // Check for 200-status error.
        if let Some(error_msg) = raw_value.get("error").and_then(|v| v.as_str()) {
            return Err(AiMuxError::ApiCall(ApiCallError {
                status_code: Some(resp.status),
                provider_code: raw_value
                    .get("code")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                message: error_msg.to_string(),
                response_body: Some(String::from_utf8_lossy(&resp.body).into_owned()),
                ..Default::default()
            }));
        }

        let data: types::XaiResponsesResponse = serde_json::from_value(raw_value.clone())?;

        let mut content: Vec<GenerateContent> = Vec::new();
        let mut has_function_call = false;

        for part in &data.output {
            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");

            // ── file_search_call ──
            if part_type == "file_search_call" {
                let tool_name = resolve_tool_name(part_type, None, &provider_tool_names);
                let part_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");

                content.push(GenerateContent::ToolCall {
                    tool_call_id: part_id.to_string(),
                    tool_name: tool_name.clone(),
                    input: Value::String(String::new()),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });

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

                content.push(GenerateContent::ToolResult {
                    tool_call_id: part_id.to_string(),
                    tool_name,
                    result: json!({ "queries": queries, "results": results }),
                    is_error: None,
                    preliminary: None,
                    dynamic: None,
                    provider_metadata: None,
                });
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

                content.push(GenerateContent::ToolCall {
                    tool_call_id: part_id.to_string(),
                    tool_name,
                    input: Value::String(tool_input),
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
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
                                    content.push(GenerateContent::Source {
                                        id: generate_source_id(),
                                        source_type: "url".to_string(),
                                        url: Some(url.to_string()),
                                        title: Some(title),
                                        provider_metadata: None,
                                    });
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
                    let input: Value = serde_json::from_str(arguments)
                        .unwrap_or_else(|_| Value::String(arguments.to_string()));
                    content.push(GenerateContent::ToolCall {
                        tool_call_id: call_id.to_string(),
                        tool_name: name.to_string(),
                        input,
                        provider_executed: None,
                        dynamic: None,
                        thought_signature: None,
                        provider_metadata: None,
                    });
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
                                        .map(|t| t.to_string())
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
                                                .map(|t| t.to_string())
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
                    let encrypted_content = part.get("encrypted_content").and_then(|v| v.as_str());
                    let item_id = part.get("id").and_then(|v| v.as_str());

                    if !reasoning_text.is_empty() || encrypted_content.is_some() {
                        let mut meta = json!({});
                        if let Some(ec) = encrypted_content {
                            meta["reasoningEncryptedContent"] = json!(ec);
                        }
                        if let Some(id) = item_id {
                            meta["itemId"] = json!(id);
                        }
                        content.push(GenerateContent::Reasoning {
                            text: reasoning_text,
                            provider_metadata: Some(json!({ "xai": meta })),
                        });
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

        let (usage, provider_metadata) = if let Some(u) = &data.usage {
            let meta = if u.cost_in_usd_ticks.is_some() {
                Some(json!({
                    "xai": { "costInUsdTicks": u.cost_in_usd_ticks }
                }))
            } else {
                None
            };
            (convert_xai_responses_usage(u), meta)
        } else {
            (Usage::default(), None)
        };

        let timestamp = data
            .created_at
            .and_then(|c| chrono::DateTime::from_timestamp(c as i64, 0).map(|dt| dt.to_rfc3339()));

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_result.warnings,
            provider_metadata,
            response: ResponseMetadata {
                id: data.id,
                timestamp,
                model_id: data.model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, true)?;
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_tool_names = request_result.provider_tool_names;

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            self.config.retry_config(),
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        // Check for JSON error (200-status).
        let content_type = response_headers
            .get("content-type")
            .map(|s| s.as_str())
            .unwrap_or("");
        if content_type.contains("application/json") {
            // Collect the (non-SSE) JSON body and check for an error object.
            let mut buf = Vec::new();
            let mut body_stream = resp.body;
            while let Some(chunk) = body_stream.next().await {
                if let Ok(bytes) = chunk {
                    buf.extend_from_slice(&bytes);
                }
            }
            let val: Value = serde_json::from_slice(&buf)?;
            if let Some(err_msg) = val.get("error").and_then(|v| v.as_str()) {
                return Err(AiMuxError::ApiCall(ApiCallError {
                    status_code: Some(resp.status),
                    provider_code: val
                        .get("code")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    message: err_msg.to_string(),
                    response_body: Some(String::from_utf8_lossy(&buf).into_owned()),
                    ..Default::default()
                }));
            }
            return Err(AiMuxError::InvalidResponseData(
                "expected SSE stream but got JSON response without an error object".to_string(),
            ));
        }

        let sse_stream = SseStream::new(resp.body);

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            let mut final_usage: Option<Usage> = None;
            let mut final_finish_reason: FinishReason = FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            };
            let mut cost_in_usd_ticks: Option<u64> = None;
            let mut has_function_call = false;
            let mut is_first_chunk = true;

            // Track content blocks.
            let mut content_blocks: HashMap<String, bool> = HashMap::new(); // block_id -> ended (text blocks)
            let mut seen_tool_calls: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut active_reasoning: HashMap<String, ()> = HashMap::new();

            // Track ongoing function calls by output_index.
            let mut ongoing_tool_calls: HashMap<u64, (String, String)> = HashMap::new(); // output_index -> (tool_call_id, tool_name)

            let mut event_iter = sse_stream;

            while let Some(event) = event_iter.next().await {
                match event {
                    Ok(sse_event) => {
                        if sse_event.data == "[DONE]" {
                            break;
                        }

                        let parsed: Value = match serde_json::from_str(&sse_event.data) {
                            Ok(v) => v,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::from(e),
                                });
                                break;
                            }
                        };

                        let event_type = types::event_type(&parsed);

                        // ── response.created / response.in_progress ──
                        if event_type == "response.created" || event_type == "response.in_progress" {
                            if is_first_chunk {
                                is_first_chunk = false;
                                let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                                let id = response.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                                let model = response.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
                                let created_at = response.get("created_at").and_then(|v| v.as_u64());
                                let timestamp = created_at.and_then(|c| {
                                    chrono::DateTime::from_timestamp(c as i64, 0).map(|dt| dt.to_rfc3339())
                                });
                                yield Ok(StreamPart::ResponseMetadata { id, timestamp, model_id: model });
                            }
                            continue;
                        }

                        // ── reasoning_summary_part.added ──
                        if event_type == "response.reasoning_summary_part.added" {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("reasoning-{}", item_id);
                            if !active_reasoning.contains_key(item_id) {
                                active_reasoning.insert(item_id.to_string(), ());
                                yield Ok(StreamPart::ReasoningStart {
                                    id: block_id,
                                    provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                                });
                            }
                            continue;
                        }

                        // ── reasoning_summary_text.delta ──
                        if event_type == "response.reasoning_summary_text.delta" {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("reasoning-{}", item_id);
                            yield Ok(StreamPart::ReasoningDelta {
                                id: block_id,
                                delta: delta.to_string(),
                                provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                            });
                            continue;
                        }

                        // ── reasoning_summary_text.done ──
                        if event_type == "response.reasoning_summary_text.done" {
                            continue;
                        }

                        // ── reasoning_text.delta ──
                        if event_type == "response.reasoning_text.delta" {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("reasoning-{}", item_id);
                            if !active_reasoning.contains_key(item_id) {
                                active_reasoning.insert(item_id.to_string(), ());
                                yield Ok(StreamPart::ReasoningStart {
                                    id: block_id.clone(),
                                    provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                                });
                            }
                            yield Ok(StreamPart::ReasoningDelta {
                                id: block_id,
                                delta: delta.to_string(),
                                provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
                            });
                            continue;
                        }

                        // ── reasoning_text.done ──
                        if event_type == "response.reasoning_text.done" {
                            continue;
                        }

                        // ── output_text.delta ──
                        if event_type == "response.output_text.delta" {
                            let item_id = parsed.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            let block_id = format!("text-{}", item_id);
                            if !content_blocks.contains_key(&block_id) {
                                content_blocks.insert(block_id.clone(), false);
                                yield Ok(StreamPart::TextStart { id: block_id.clone(), provider_metadata: None});
                            }
                            yield Ok(StreamPart::TextDelta {
                                id: block_id,
                                delta: delta.to_string(),
                                provider_metadata: None,
                            });
                            continue;
                        }

                        // ── output_text.done ──
                        if event_type == "response.output_text.done" {
                            if let Some(annotations) = parsed.get("annotations").and_then(|v| v.as_array()) {
                                for ann in annotations {
                                    if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                        let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                        let title = ann.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string();
                                        yield Ok(StreamPart::Source {
                                            id: generate_source_id(),
                                            source_type: "url".to_string(),
                                            url: Some(url.to_string()),
                                            title: Some(title),
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            }
                            continue;
                        }

                        // ── output_text.annotation.added ──
                        if event_type == "response.output_text.annotation.added" {
                            let annotation = parsed.get("annotation").cloned().unwrap_or(Value::Null);
                            if annotation.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                let url = annotation.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                let title = annotation.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string();
                                yield Ok(StreamPart::Source {
                                    id: generate_source_id(),
                                    source_type: "url".to_string(),
                                    url: Some(url.to_string()),
                                    title: Some(title),
                                    provider_metadata: None,
                                });
                            }
                            continue;
                        }

                        // ── response.done / response.completed / response.incomplete ──
                        if event_type == "response.done"
                            || event_type == "response.completed"
                            || event_type == "response.incomplete"
                        {
                            let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                            if let Some(usage) = response.get("usage")
                                && let Ok(u) = serde_json::from_value::<types::XaiResponsesUsage>(usage.clone()) {
                                    final_usage = Some(convert_xai_responses_usage(&u));
                                    cost_in_usd_ticks = u.cost_in_usd_ticks;
                                }

                            if event_type == "response.incomplete" {
                                let reason = response
                                    .get("incomplete_details")
                                    .and_then(|d| d.get("reason"))
                                    .and_then(|v| v.as_str());
                                final_finish_reason = FinishReason {
                                    unified: reason
                                        .map(map_xai_responses_finish_reason)
                                        .unwrap_or(FinishReasonUnified::Other),
                                    raw: reason.map(|s| s.to_string()).or(Some("incomplete".to_string())),
                                };
                            } else if let Some(status) = response.get("status").and_then(|v| v.as_str()) {
                                final_finish_reason = FinishReason {
                                    unified: if has_function_call {
                                        FinishReasonUnified::ToolCalls
                                    } else {
                                        map_xai_responses_finish_reason(status)
                                    },
                                    raw: Some(status.to_string()),
                                };
                            }
                            continue;
                        }

                        // ── response.failed ──
                        if event_type == "response.failed" {
                            let response = parsed.get("response").cloned().unwrap_or(Value::Null);
                            let reason = response
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|v| v.as_str());
                            final_finish_reason = FinishReason {
                                unified: reason
                                    .map(map_xai_responses_finish_reason)
                                    .unwrap_or(FinishReasonUnified::Error),
                                raw: reason.map(|s| s.to_string()).or(Some("error".to_string())),
                            };
                            if let Some(usage) = response.get("usage")
                                && let Ok(u) = serde_json::from_value::<types::XaiResponsesUsage>(usage.clone()) {
                                    final_usage = Some(convert_xai_responses_usage(&u));
                                }
                            continue;
                        }

                        // ── error event ──
                        if event_type == "error" {
                            let message = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                            yield Ok(StreamPart::Error {
                                error: AiMuxError::ApiCall(ApiCallError {
                                    provider_code: parsed.get("code").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                    message: message.to_string(),
                                    response_body: Some(sse_event.data.clone()),
                                    ..Default::default()
                                }),
                            });
                            continue;
                        }

                        // ── custom_tool_call_input.delta / .done ──
                        if event_type == "response.custom_tool_call_input.delta"
                            || event_type == "response.custom_tool_call_input.done"
                        {
                            continue;
                        }

                        // ── function_call_arguments.delta ──
                        if event_type == "response.function_call_arguments.delta" {
                            let output_index = parsed.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);
                            let delta = parsed.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some((tool_call_id, _)) = ongoing_tool_calls.get(&output_index) {
                                yield Ok(StreamPart::ToolInputDelta {
                                    id: tool_call_id.clone(),
                                    delta: delta.to_string(),
                                    provider_metadata: None,
                                });
                            }
                            continue;
                        }

                        // ── function_call_arguments.done ──
                        if event_type == "response.function_call_arguments.done" {
                            continue;
                        }

                        // ── output_item.added / output_item.done ──
                        if event_type == "response.output_item.added"
                            || event_type == "response.output_item.done"
                        {
                            let item = parsed.get("item").cloned().unwrap_or(Value::Null);
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let output_index = parsed.get("output_index").and_then(|v| v.as_u64()).unwrap_or(0);

                            // ── reasoning item ──
                            if item_type == "reasoning" {
                                if event_type == "response.output_item.done" {
                                    let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    let block_id = format!("reasoning-{}", part_id);
                                    let encrypted = item.get("encrypted_content").and_then(|v| v.as_str());

                                    if !active_reasoning.contains_key(part_id) {
                                        active_reasoning.insert(part_id.to_string(), ());
                                        yield Ok(StreamPart::ReasoningStart {
                                            id: block_id.clone(),
                                            provider_metadata: Some(json!({ "xai": { "itemId": part_id } })),
                                        });
                                    }

                                    let mut meta = json!({ "itemId": part_id });
                                    if let Some(ec) = encrypted {
                                        meta["reasoningEncryptedContent"] = json!(ec);
                                    }
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: block_id,
                                        provider_metadata: Some(json!({ "xai": meta })),
                                    });
                                    active_reasoning.remove(part_id);
                                }
                                continue;
                            }

                            // ── file_search_call item ──
                            if item_type == "file_search_call" {
                                let tool_name = resolve_tool_name(item_type, None, &provider_tool_names);
                                let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");

                                if !seen_tool_calls.contains(part_id) {
                                    seen_tool_calls.insert(part_id.to_string());
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: part_id.to_string(),
                                        delta: String::new(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: part_id.to_string(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        input: Value::String(String::new()),
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                }

                                if event_type == "response.output_item.done" {
                                    let queries = item.get("queries").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                    let results = item.get("results").and_then(|v| v.as_array()).map(|arr| {
                                        json!(arr.iter().map(|r| json!({
                                            "fileId": r.get("file_id").cloned().unwrap_or(Value::Null),
                                            "filename": r.get("filename").cloned().unwrap_or(Value::Null),
                                            "score": r.get("score").cloned().unwrap_or(Value::Null),
                                            "text": r.get("text").cloned().unwrap_or(Value::Null),
                                        })).collect::<Vec<_>>())
                                    }).unwrap_or(Value::Null);
                                    yield Ok(StreamPart::ToolResult {
                                        tool_call_id: part_id.to_string(),
                                        tool_name,
                                        result: json!({ "queries": queries, "results": results }),
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: None,
                                    });
                                }
                                continue;
                            }

                            // ── Server-side tool calls ──
                            if matches!(
                                item_type,
                                "web_search_call" | "x_search_call" | "code_interpreter_call"
                                    | "code_execution_call" | "view_image_call" | "view_x_video_call"
                                    | "custom_tool_call" | "mcp_call"
                            ) {
                                let item_name = item.get("name").and_then(|v| v.as_str());
                                let tool_name = resolve_tool_name(item_type, item_name, &provider_tool_names);
                                let tool_input = get_tool_input(item_type, &item);
                                let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");

                                let should_emit = if item_type == "custom_tool_call" {
                                    event_type == "response.output_item.done"
                                } else {
                                    !seen_tool_calls.contains(part_id)
                                };

                                if should_emit && !seen_tool_calls.contains(part_id) {
                                    seen_tool_calls.insert(part_id.to_string());
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: part_id.to_string(),
                                        delta: tool_input.clone(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: part_id.to_string(),
                                        provider_metadata: None,
                                    });
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: part_id.to_string(),
                                        tool_name: tool_name.clone(),
                                        input: Value::String(tool_input),
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                }

                                if event_type == "response.output_item.done" {
                                    yield Ok(StreamPart::ToolResult {
                                        tool_call_id: part_id.to_string(),
                                        tool_name,
                                        result: json!({}),
                                        is_error: None,
                                        preliminary: None,
                                        dynamic: None,
                                        provider_metadata: None,
                                    });
                                }
                                continue;
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
                                        && !text.is_empty() {
                                            let part_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                            let block_id = format!("text-{}", part_id);
                                            if !content_blocks.contains_key(&block_id) {
                                                content_blocks.insert(block_id.clone(), false);
                                                yield Ok(StreamPart::TextStart { id: block_id.clone(), provider_metadata: None});
                                                yield Ok(StreamPart::TextDelta {
                                                    id: block_id,
                                                    delta: text.to_string(),
                                                    provider_metadata: None,
                                                });
                                            }
                                        }
                                    if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array()) {
                                        for ann in annotations {
                                            if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation") {
                                                let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                                                let title = ann.get("title").and_then(|v| v.as_str()).unwrap_or(url).to_string();
                                                yield Ok(StreamPart::Source {
                                                    id: generate_source_id(),
                                                    source_type: "url".to_string(),
                                                    url: Some(url.to_string()),
                                                    title: Some(title),
                                                    provider_metadata: None,
                                                });
                                            }
                                        }
                                    }
                                }
                                continue;
                            }

                            // ── function_call item ──
                            if item_type == "function_call" {
                                let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");

                                if event_type == "response.output_item.added" {
                                    ongoing_tool_calls.insert(output_index, (call_id.to_string(), name.to_string()));
                                    yield Ok(StreamPart::ToolInputStart {
                                        id: call_id.to_string(),
                                        tool_name: name.to_string(),
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                } else {
                                    // output_item.done
                                    has_function_call = true;
                                    ongoing_tool_calls.remove(&output_index);
                                    let arguments = item.get("arguments").and_then(|v| v.as_str()).unwrap_or("");
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: call_id.to_string(),
                                        provider_metadata: None,
                                    });
                                    let input: Value = serde_json::from_str(arguments)
                                        .unwrap_or_else(|_| Value::String(arguments.to_string()));
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: call_id.to_string(),
                                        tool_name: name.to_string(),
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                }
                                continue;
                            }
                        }

                        // All other event types (web_search_call.in_progress, etc.) are ignored.
                    }
                    Err(e) => {
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::InvalidResponseData(e.to_string()),
                        });
                        break;
                    }
                }
            }

            // Close any remaining open text blocks.
            for (block_id, ended) in &content_blocks {
                if !ended {
                    yield Ok(StreamPart::TextEnd { id: block_id.clone(), provider_metadata: None});
                }
            }

            // Final part: Finish.
            let provider_meta = cost_in_usd_ticks.map(|cost| json!({ "xai": { "costInUsdTicks": cost } }));

            yield Ok(StreamPart::Finish {
                finish_reason: final_finish_reason,
                usage: final_usage.unwrap_or(Usage {
                    input_tokens: aimux_core::types::TokenUsage {
                        total: Some(0),
                        no_cache: Some(0),
                        cache_read: Some(0),
                        cache_write: Some(0),
                        ..Default::default()
                    },
                    output_tokens: aimux_core::types::TokenUsage {
                        total: Some(0),
                        text: Some(0),
                        reasoning: Some(0),
                        ..Default::default()
                    },
                    raw: None,
                }),
                provider_metadata: provider_meta,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}
