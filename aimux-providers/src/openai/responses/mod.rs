//! OpenAI Responses API language model.
//!
//! Implements the [`LanguageModel`] trait against the `/v1/responses`
//! endpoint. The Responses API uses a different request/response format from
//! chat completions: requests carry an `input` array (not `messages`), and
//! streaming events are typed as `response.output_text.delta`,
//! `response.function_call_arguments.delta`, etc.
//!
//! Mirrors the TS `OpenAIResponsesLanguageModel`. The core paths implemented
//! here are:
//! - **Request building**: input array, instructions, store,
//!   previous_response_id, reasoning (effort/summary), response_format
//!   (json_schema/json_object) — see [`convert::build_responses_request_body`].
//! - **Non-streaming**: text, function-call, and reasoning output items -- see
//!   [`OpenAIResponsesModel::do_generate`].
//! - **Streaming**: the `response.created -> output_item.added ->
//!   output_text.delta -> output_text.done -> output_item.done ->
//!   response.completed` main path, plus `function_call_arguments.delta/done`
//!   and `reasoning_summary_text.delta` -- see [`OpenAIResponsesModel::do_stream`].

pub mod convert;
pub mod types;

pub use convert::{
    ResponsesInputResult, ResponsesRequestBodyResult, build_responses_request_body,
    convert_responses_usage, convert_to_responses_input, map_responses_finish_reason,
    prepare_responses_tools,
};

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send, send_stream};
use aimux_stream::SseStream;

use super::OpenAIConfig;
use types::ResponsesUsage;

/// An OpenAI Responses API language model.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use
/// the process-wide shared `Client` internally (RFC-0009 §4.1).
///
/// Created via [`OpenAIResponsesProvider`](super::OpenAIResponsesProvider) or
/// directly with [`OpenAIResponsesModel::new`].
pub struct OpenAIResponsesModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAIResponsesModel {
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.base_url)
    }

    /// The provider-metadata key: `"azure"` when the provider string contains
    /// `"azure"`, otherwise `"openai"`. Mirrors the TS `providerOptionsName`.
    fn provider_options_name(&self) -> &str {
        if self.config.provider.contains("azure") {
            "azure"
        } else {
            "openai"
        }
    }
}

/// Build the header list for a JSON POST: auth/extra headers + `Content-Type`.
///
/// Returns a `Vec<(String, String)>` for `HttpRequest` — no reqwest types.
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> =
        headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

#[async_trait]
impl LanguageModel for OpenAIResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, false);
        let body = request_result.body;
        let provider_key = self.provider_options_name().to_string();

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let data: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Top-level error field.
        if let Some(err_obj) = data.get("error")
            && err_obj.is_object()
        {
            let message = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Responses API error");
            return Err(AiMuxError::Provider(message.to_string()));
        }

        let output = data.get("output").and_then(|v| v.as_array());
        let output = output.ok_or_else(|| {
            let detail = data
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            AiMuxError::Provider(if detail.is_empty() {
                "Responses API returned no output".to_string()
            } else {
                format!("Responses API returned no output ({})", detail)
            })
        })?;

        let mut content: Vec<GenerateContent> = Vec::new();
        let mut has_function_call = false;

        for part in output {
            match part.get("type").and_then(|v| v.as_str()) {
                Some("message") => {
                    if let Some(content_parts) = part.get("content").and_then(|v| v.as_array()) {
                        for cp in content_parts {
                            if cp.get("type").and_then(|v| v.as_str()) == Some("output_text") {
                                let text = cp
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if !text.is_empty() {
                                    content.push(GenerateContent::Text { text });
                                }
                            }
                            // Annotations (url_citation → Source).
                            if let Some(annotations) =
                                cp.get("annotations").and_then(|v| v.as_array())
                            {
                                for (i, ann) in annotations.iter().enumerate() {
                                    if ann.get("type").and_then(|v| v.as_str())
                                        == Some("url_citation")
                                    {
                                        content.push(GenerateContent::Source {
                                            id: format!("annotation-{}", i),
                                            source_type: "url".to_string(),
                                            url: ann
                                                .get("url")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                            title: ann
                                                .get("title")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                Some("function_call") => {
                    has_function_call = true;
                    let call_id = part
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = part
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = part
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    let input: Value = serde_json::from_str(&arguments)
                        .unwrap_or_else(|_| Value::String(arguments.clone()));
                    content.push(GenerateContent::ToolCall {
                        tool_call_id: call_id,
                        tool_name: name,
                        input,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: None,
                    });
                }
                Some("custom_tool_call") => {
                    has_function_call = true;
                    let call_id = part
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = part
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input_str = part.get("input").and_then(|v| v.as_str()).unwrap_or("{}");
                    let input: Value = serde_json::from_str(input_str)
                        .unwrap_or_else(|_| Value::String(input_str.to_string()));
                    content.push(GenerateContent::ToolCall {
                        tool_call_id: call_id,
                        tool_name: name,
                        input,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: None,
                    });
                }
                Some("reasoning") => {
                    let summary = part.get("summary").and_then(|v| v.as_array());
                    let parts: Vec<&Value> =
                        summary.map(|s| s.iter().collect()).unwrap_or_default();
                    if parts.is_empty() {
                        content.push(GenerateContent::Reasoning {
                            text: String::new(),
                            provider_metadata: None,
                        });
                    } else {
                        for sp in parts {
                            let text = sp
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            content.push(GenerateContent::Reasoning {
                                text,
                                provider_metadata: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        let incomplete_reason = data
            .get("incomplete_details")
            .and_then(|d| d.get("reason"))
            .and_then(|v| v.as_str());
        let finish_reason = map_responses_finish_reason(incomplete_reason, has_function_call);

        let usage =
            convert_responses_usage(data.get("usage").and_then(convert::parse_usage).as_ref());

        // Provider metadata: { <provider_key>: { responseId, reasoningContext?, serviceTier? } }
        let mut pm = json!({ "responseId": data.get("id").cloned().unwrap_or(Value::Null) });
        if let Some(reasoning) = data.get("reasoning")
            && let Some(ctx) = reasoning.get("context")
            && !ctx.is_null()
        {
            pm["reasoningContext"] = ctx.clone();
        }
        if let Some(st) = data.get("service_tier").and_then(|v| v.as_str()) {
            pm["serviceTier"] = json!(st);
        }
        let provider_metadata = Some(json!({ provider_key: pm }));

        let response_id = data
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let model = data
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let timestamp = data
            .get("created_at")
            .and_then(|v| v.as_u64())
            .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
            .map(|dt| dt.to_rfc3339());

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_result.warnings,
            provider_metadata,
            response: ResponseMetadata {
                id: response_id,
                timestamp,
                model_id: model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, true);
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_key = self.provider_options_name().to_string();

        // The `store` request option (None by default). Used to decide when
        // reasoning summary parts are concluded.
        let store_flag = options
            .provider_options
            .as_ref()
            .and_then(|m| m.get("openai"))
            .and_then(|o| o.get("store"))
            .and_then(|v| v.as_bool())
            == Some(true);

        let resp = send_stream(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let mut sse_stream = SseStream::new(resp.body);

        // Peek at the first SSE event to detect early errors (before any output).
        let first_event = sse_stream.next().await;
        if let Some(Ok(ref event)) = first_event
            && let Ok(val) = serde_json::from_str::<Value>(&event.data)
        {
            let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if etype == "error" || etype == "response.failed" {
                let message = val
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        val.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(|v| v.as_str())
                    })
                    .unwrap_or("Responses API stream error");
                return Err(AiMuxError::Provider(message.to_string()));
            }
        }

        let provider_key_stream = provider_key.clone();
        let stream = async_stream::stream! {
            // First part: StreamStart.
            yield Ok(StreamPart::StreamStart { warnings });

            // Ongoing tool calls keyed by output_index.
            let mut ongoing_tool_calls: HashMap<usize, OngoingToolCall> = HashMap::new();
            // Active reasoning items keyed by item_id.
            let mut active_reasoning: HashMap<String, ReasoningState> = HashMap::new();

            let mut has_function_call = false;
            let mut final_usage: Option<ResponsesUsage> = None;
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut response_id: Option<String> = None;
            let mut stream_errored = false;

            let mut event_iter =
                futures::stream::iter(first_event.into_iter()).chain(sse_stream);

            while let Some(event) = event_iter.next().await {
                if stream_errored {
                    break;
                }

                match event {
                    Ok(sse_event) => {
                        if sse_event.data == "[DONE]" {
                            break;
                        }

                        let parsed: Value = match serde_json::from_str(&sse_event.data) {
                            Ok(v) => v,
                            Err(e) => {
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                stream_errored = true;
                                break;
                            }
                        };

                        let etype = parsed
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        match etype.as_str() {
                            // ── response.created → ResponseMetadata ───────────────────
                            "response.created" => {
                                if let Some(resp_obj) = parsed.get("response") {
                                    response_id = resp_obj
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let model_id = resp_obj
                                        .get("model")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let timestamp = resp_obj
                                        .get("created_at")
                                        .and_then(|v| v.as_u64())
                                        .and_then(|secs| {
                                            chrono::DateTime::from_timestamp(secs as i64, 0)
                                        })
                                        .map(|dt| dt.to_rfc3339());
                                    yield Ok(StreamPart::ResponseMetadata {
                                        id: response_id.clone(),
                                        timestamp,
                                        model_id,
                                    });
                                }
                            }

                            // ── response.output_item.added ─────────────────────────────
                            "response.output_item.added" => {
                                let output_index = parsed
                                    .get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                                if let Some(item) = parsed.get("item") {
                                    let item_type = item
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    match item_type {
                                        "message" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            yield Ok(StreamPart::TextStart { id });
                                        }
                                        "function_call" => {
                                            let call_id = item
                                                .get("call_id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            ongoing_tool_calls.insert(
                                                output_index,
                                                OngoingToolCall {
                                                    tool_name: name.clone(),
                                                    tool_call_id: call_id.clone(),
                                                },
                                            );
                                            yield Ok(StreamPart::ToolInputStart {
                                                id: call_id,
                                                tool_name: name,
                                                provider_executed: None,
                                                dynamic: None,
                                            });
                                        }
                                        "custom_tool_call" => {
                                            let call_id = item
                                                .get("call_id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            ongoing_tool_calls.insert(
                                                output_index,
                                                OngoingToolCall {
                                                    tool_name: name.clone(),
                                                    tool_call_id: call_id.clone(),
                                                },
                                            );
                                            yield Ok(StreamPart::ToolInputStart {
                                                id: call_id,
                                                tool_name: name,
                                                provider_executed: None,
                                                dynamic: None,
                                            });
                                        }
                                        "reasoning" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let encrypted = item
                                                .get("encrypted_content")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string());
                                            active_reasoning.insert(
                                                id.clone(),
                                                ReasoningState {
                                                    encrypted_content: encrypted,
                                                    summary_parts: HashMap::from([(
                                                        0usize,
                                                        SummaryStatus::Active,
                                                    )]),
                                                },
                                            );
                                            yield Ok(StreamPart::ReasoningStart {
                                                id: format!("{}:0", id),
                                                provider_metadata: None,
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            // ── response.output_text.delta → TextDelta ────────────────
                            "response.output_text.delta" => {
                                let id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let delta = parsed
                                    .get("delta")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                yield Ok(StreamPart::TextDelta { id, delta });
                            }

                            // ── function_call_arguments.delta → ToolInputDelta ────────
                            "response.function_call_arguments.delta"
                            | "response.custom_tool_call_input.delta" => {
                                let output_index = parsed
                                    .get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                                let delta = parsed
                                    .get("delta")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if let Some(tc) = ongoing_tool_calls.get(&output_index) {
                                    yield Ok(StreamPart::ToolInputDelta {
                                        id: tc.tool_call_id.clone(),
                                        delta,
                                    });
                                }
                            }

                            // ── reasoning_summary_part.added ──────────────────────────
                            "response.reasoning_summary_part.added" => {
                                let item_id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let summary_index = parsed
                                    .get("summary_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;

                                if summary_index > 0
                                    && let Some(state) = active_reasoning.get_mut(&item_id)
                                {
                                    // Conclude all 'can-conclude' parts.
                                    let to_conclude: Vec<usize> = state
                                        .summary_parts
                                        .iter()
                                        .filter(|(_, s)| **s == SummaryStatus::CanConclude)
                                        .map(|(k, _)| *k)
                                        .collect();
                                    for idx in to_conclude {
                                        state.summary_parts.insert(idx, SummaryStatus::Concluded);
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: format!("{}:{}", item_id, idx),
                                            provider_metadata: None,
                                        });
                                    }
                                    state
                                        .summary_parts
                                        .insert(summary_index, SummaryStatus::Active);
                                    let encrypted = state.encrypted_content.clone();
                                    yield Ok(StreamPart::ReasoningStart {
                                        id: format!("{}:{}", item_id, summary_index),
                                        provider_metadata: None,
                                    });
                                    let _ = encrypted;
                                }
                            }

                            // ── reasoning_summary_text.delta → ReasoningDelta ─────────
                            "response.reasoning_summary_text.delta" => {
                                let item_id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let summary_index = parsed
                                    .get("summary_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                                let delta = parsed
                                    .get("delta")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: format!("{}:{}", item_id, summary_index),
                                    delta,
                                    provider_metadata: None,
                                });
                            }

                            // ── reasoning_summary_part.done ───────────────────────────
                            "response.reasoning_summary_part.done" => {
                                let item_id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let summary_index = parsed
                                    .get("summary_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                                if let Some(state) = active_reasoning.get_mut(&item_id) {
                                    if store_flag {
                                        state
                                            .summary_parts
                                            .insert(summary_index, SummaryStatus::Concluded);
                                        yield Ok(StreamPart::ReasoningEnd {
                                            id: format!("{}:{}", item_id, summary_index),
                                            provider_metadata: None,
                                        });
                                    } else {
                                        state
                                            .summary_parts
                                            .insert(summary_index, SummaryStatus::CanConclude);
                                    }
                                }
                            }

                            // ── response.output_item.done ─────────────────────────────
                            "response.output_item.done" => {
                                let output_index = parsed
                                    .get("output_index")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0) as usize;
                                if let Some(item) = parsed.get("item") {
                                    let item_type = item
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    match item_type {
                                        "message" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            yield Ok(StreamPart::TextEnd { id });
                                        }
                                        "function_call" => {
                                            has_function_call = true;
                                            ongoing_tool_calls.remove(&output_index);
                                            let call_id = item
                                                .get("call_id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let arguments = item
                                                .get("arguments")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("{}")
                                                .to_string();
                                            yield Ok(StreamPart::ToolInputEnd {
                                                id: call_id.clone(),
                                            });
                                            let input: Value = serde_json::from_str(&arguments)
                                                .unwrap_or_else(|_| {
                                                    Value::String(arguments.clone())
                                                });
                                            yield Ok(StreamPart::ToolCall {
                                                tool_call_id: call_id,
                                                tool_name: name,
                                                input,
                                                provider_executed: None,
                                                dynamic: None,
                                            });
                                        }
                                        "custom_tool_call" => {
                                            has_function_call = true;
                                            ongoing_tool_calls.remove(&output_index);
                                            let call_id = item
                                                .get("call_id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let input_str = item
                                                .get("input")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("{}");
                                            yield Ok(StreamPart::ToolInputEnd {
                                                id: call_id.clone(),
                                            });
                                            let input: Value = serde_json::from_str(input_str)
                                                .unwrap_or_else(|_| {
                                                    Value::String(input_str.to_string())
                                                });
                                            yield Ok(StreamPart::ToolCall {
                                                tool_call_id: call_id,
                                                tool_name: name,
                                                input,
                                                provider_executed: None,
                                                dynamic: None,
                                            });
                                        }
                                        "reasoning" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            if let Some(state) = active_reasoning.get_mut(&id) {
                                                // Conclude all active / can-conclude parts.
                                                let to_conclude: Vec<usize> = state
                                                    .summary_parts
                                                    .iter()
                                                    .filter(|(_, s)| {
                                                        **s == SummaryStatus::Active
                                                            || **s == SummaryStatus::CanConclude
                                                    })
                                                    .map(|(k, _)| *k)
                                                    .collect();
                                                for idx in to_conclude {
                                                    state
                                                        .summary_parts
                                                        .insert(idx, SummaryStatus::Concluded);
                                                    yield Ok(StreamPart::ReasoningEnd {
                                                        id: format!("{}:{}", id, idx),
                                                        provider_metadata: None,
                                                    });
                                                }
                                            }
                                            active_reasoning.remove(&id);
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            // ── response.completed / response.incomplete → finish ────
                            "response.completed" | "response.incomplete" => {
                                if let Some(resp_obj) = parsed.get("response") {
                                    let reason = resp_obj
                                        .get("incomplete_details")
                                        .and_then(|d| d.get("reason"))
                                        .and_then(|v| v.as_str());
                                    final_finish_reason = Some(map_responses_finish_reason(
                                        reason,
                                        has_function_call,
                                    ));
                                    final_usage =
                                        resp_obj.get("usage").and_then(convert::parse_usage);
                                }
                            }

                            // ── response.failed ───────────────────────────────────────
                            "response.failed" => {
                                if let Some(resp_obj) = parsed.get("response") {
                                    let reason = resp_obj
                                        .get("incomplete_details")
                                        .and_then(|d| d.get("reason"))
                                        .and_then(|v| v.as_str());
                                    final_finish_reason = Some(match reason {
                                        Some(r) => map_responses_finish_reason(Some(r), has_function_call),
                                        None => FinishReason {
                                            unified: FinishReasonUnified::Error,
                                            raw: Some("error".to_string()),
                                        },
                                    });
                                    final_usage =
                                        resp_obj.get("usage").and_then(convert::parse_usage);
                                    if !stream_errored
                                        && resp_obj.get("error").is_some()
                                    {
                                        stream_errored = true;
                                        let message = resp_obj
                                            .get("error")
                                            .and_then(|e| e.get("message"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Responses API stream failed");
                                        yield Ok(StreamPart::Error {
                                            error: AiMuxError::Provider(message.to_string()),
                                        });
                                    }
                                }
                            }

                            // ── error chunk ───────────────────────────────────────────
                            "error" => {
                                stream_errored = true;
                                final_finish_reason = Some(FinishReason {
                                    unified: FinishReasonUnified::Error,
                                    raw: Some("error".to_string()),
                                });
                                let message = parsed
                                    .get("error")
                                    .and_then(|e| e.get("message"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Responses API stream error");
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Provider(message.to_string()),
                                });
                            }

                            _ => {
                                // Unknown / unhandled chunk types (content_part.added,
                                // output_text.done, content_part.done, etc.) are
                                // ignored — the core streaming path is driven by
                                // output_item.added/done and the delta events.
                            }
                        }
                    }
                    Err(e) => {
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::Stream(e.to_string()),
                        });
                        stream_errored = true;
                        break;
                    }
                }
            }

            // Build provider metadata for the Finish part.
            let mut pm = json!({ "responseId": response_id.unwrap_or_default() });
            let provider_metadata = Some(json!({ provider_key_stream: pm }));
            let _ = &mut pm;

            yield Ok(StreamPart::Finish {
                finish_reason: if stream_errored {
                    FinishReason {
                        unified: FinishReasonUnified::Error,
                        raw: None,
                    }
                } else {
                    final_finish_reason.unwrap_or(FinishReason {
                        unified: if has_function_call {
                            FinishReasonUnified::ToolCalls
                        } else {
                            FinishReasonUnified::Stop
                        },
                        raw: None,
                    })
                },
                usage: if stream_errored {
                    Usage::default()
                } else {
                    convert_responses_usage(final_usage.as_ref())
                },
                provider_metadata,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

// ── Streaming state helpers ─────────────────────────────────────────────────

/// A tool call being streamed (tracked by `output_index`).
#[allow(dead_code)]
struct OngoingToolCall {
    tool_name: String,
    tool_call_id: String,
}

/// A reasoning item being streamed (tracked by `item_id`).
struct ReasoningState {
    encrypted_content: Option<String>,
    /// summary_index → status.
    summary_parts: HashMap<usize, SummaryStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SummaryStatus {
    Active,
    CanConclude,
    Concluded,
}
