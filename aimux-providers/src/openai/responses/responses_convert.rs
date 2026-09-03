//! Shared Responses API framework (RFC-0012 §3.5).
//!
//! Vendors whose Responses implementations speak the OpenAI wire format
//! (currently OpenAI and Azure OpenAI) share this module for the parts that are
//! byte-identical across them:
//! - non-streaming output parsing — [`build_responses_generate_result`],
//! - the streaming SSE event reducer — [`build_responses_event_stream`],
//! - common HTTP header list construction — [`build_header_list`].
//!
//! Vendors with genuinely different protocols (xAI, HuggingFace, the generic
//! `open_responses` provider) keep their own request/streaming logic and reuse
//! only the small shared helpers where they are byte-identical. Per the RFC,
//! genuinely different streaming loops are **not** force-merged into one
//! function — only the shared framework is extracted.

use std::collections::HashMap;
use std::pin::Pin;

use futures::{Stream, StreamExt};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage, Warning};
use aimux_stream::{SseError, SseEvent};

use super::convert::{convert_responses_usage, map_responses_finish_reason, parse_usage};
use super::types::ResponsesUsage;

/// Pinned, boxed stream of model stream parts.
///
/// Matches the `stream` field of [`aimux_core::result::StreamResult`]. Used as
/// the return type of the shared streaming reducer so the boxed trait object
/// does not leak a complex type into call sites.
pub type ResponsesEventStream = Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>;

/// Build the `Vec<(String, String)>` header list for an `HttpRequest`, appending
/// `Content-Type: application/json`.
///
/// Byte-identical copies previously lived in the OpenAI, HuggingFace and xAI
/// responses modules; they now route through this single implementation.
#[must_use]
pub fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

// -- Non-streaming output parsing --------------------------------------------

/// Parse a non-streaming Responses API JSON body into a [`GenerateResult`].
///
/// Shared verbatim by the OpenAI and Azure providers: both speak the same
/// Responses wire format for non-streaming output (top-level error, `output`
/// array of `message`/`function_call`/`custom_tool_call`/`reasoning` items,
/// `incomplete_details`, `usage`, provider metadata with `responseId` /
/// `reasoningContext` / `serviceTier`). Vendor callers supply the parsed `data`,
/// the observed HTTP `status` and full `raw_body` (evidence for in-band 2xx
/// errors), the request `body`/`response_headers` to attach, and the
/// provider-metadata namespace `provider_key` ("openai" / "azure").
///
/// # Errors
///
/// Returns `ApiCall` for in-band 2xx errors (top-level `error` object, error
/// status, `incomplete_details`) and `InvalidResponseData` when required
/// fields such as `output` are missing.
pub fn build_responses_generate_result(
    data: &Value,
    status: u16,
    raw_body: &str,
    request_warnings: Vec<Warning>,
    provider_key: String,
    body: Value,
    response_headers: HashMap<String, String>,
) -> Result<GenerateResult, AiMuxError> {
    // Top-level error field.
    if let Some(err_obj) = data.get("error")
        && err_obj.is_object()
    {
        let message = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Responses API error");
        let provider_code = err_obj
            .get("type")
            .or_else(|| err_obj.get("code"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        return Err(AiMuxError::ApiCall(ApiCallError {
            // Provider-declared in-band failure: keep the observed 2xx
            // envelope status and the full raw body (§2.2).
            status_code: Some(status),
            provider_code,
            message: message.to_string(),
            response_body: Some(raw_body.to_string()),
            ..Default::default()
        }));
    }

    let output = data.get("output").and_then(|v| v.as_array());
    let output = output.ok_or_else(|| {
        let detail = data
            .get("incomplete_details")
            .and_then(|d| d.get("reason"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // A success response that cannot yield a usable result (§2.2).
        AiMuxError::InvalidResponseData(if detail.is_empty() {
            "Responses API returned no output".to_string()
        } else {
            format!("Responses API returned no output ({detail})")
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
                                content.push(GenerateContent::Text {
                                    text,
                                    provider_metadata: Some(json!({
                                        (provider_key.clone()): {
                                            "itemId": part.get("id").cloned().unwrap_or(Value::Null),
                                        }
                                    })),
                                });
                            }
                        }
                        // Annotations (url_citation → Source).
                        if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array())
                        {
                            for (i, ann) in annotations.iter().enumerate() {
                                if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation")
                                {
                                    content.push(GenerateContent::Source {
                                        id: format!("annotation-{i}"),
                                        source_type: "url".to_string(),
                                        url: ann
                                            .get("url")
                                            .and_then(|v| v.as_str())
                                            .map(std::string::ToString::to_string),
                                        title: ann
                                            .get("title")
                                            .and_then(|v| v.as_str())
                                            .map(std::string::ToString::to_string),
                                        provider_metadata: None,
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
                let input = arguments;
                content.push(GenerateContent::ToolCall {
                    tool_call_id: call_id,
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: Some(json!({
                        (provider_key.clone()): {
                            "itemId": part.get("id").cloned().unwrap_or(Value::Null),
                        }
                    })),
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
                let input = serde_json::to_string(input_str)
                    .expect("serializing a custom-tool input string cannot fail");
                content.push(GenerateContent::ToolCall {
                    tool_call_id: call_id,
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: Some(json!({
                        (provider_key.clone()): {
                            "itemId": part.get("id").cloned().unwrap_or(Value::Null),
                        }
                    })),
                });
            }
            Some("reasoning") => {
                let summary = part.get("summary").and_then(|v| v.as_array());
                let parts: Vec<&Value> = summary.map(|s| s.iter().collect()).unwrap_or_default();
                let reasoning_metadata = Some(json!({
                    (provider_key.clone()): {
                        "itemId": part.get("id").cloned().unwrap_or(Value::Null),
                        "reasoningEncryptedContent": part.get("encrypted_content").cloned().unwrap_or(Value::Null),
                    }
                }));
                if parts.is_empty() {
                    content.push(GenerateContent::Reasoning {
                        text: String::new(),
                        provider_metadata: reasoning_metadata.clone(),
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
                            provider_metadata: reasoning_metadata.clone(),
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

    let usage = convert_responses_usage(
        data.get("usage").and_then(parse_usage).as_ref(),
        data.get("usage").cloned(),
    );

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
        .map(std::string::ToString::to_string);
    let model = data
        .get("model")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let timestamp = data
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
        .map(|dt| dt.to_rfc3339());

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings: request_warnings,
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

// -- Streaming SSE event reducer ---------------------------------------------

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

/// Provider metadata for streamed reasoning parts, in the same shape the
/// non-streaming path uses (`{ <provider_key>: { itemId,
/// reasoningEncryptedContent? } }`) so `consume()` can propagate it into
/// `response_messages` and the next turn's request conversion can read it
/// back (`item_reference` when stored, `encrypted_content` when not).
fn reasoning_stream_metadata(
    provider_key: &str,
    item_id: &str,
    encrypted_content: Option<&str>,
) -> Value {
    let mut inner = json!({ "itemId": item_id });
    if let Some(enc) = encrypted_content {
        inner["reasoningEncryptedContent"] = json!(enc);
    }
    json!({ (provider_key): inner })
}

/// Generate a unique source ID for streaming annotation sources.
///
/// Uses a process-wide atomic counter — consistent with the xAI Responses
/// provider (`aimux-providers/src/xai/responses/mod.rs`). Upstream TS uses
/// `generateId()` for the same purpose.
fn generate_source_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("source-{n}")
}

/// Build the streaming event reducer shared by the OpenAI and Azure providers.
///
/// Both speak the same Responses streaming wire format (the
/// `response.created -> output_item.added -> output_text.delta ->
/// output_item.done -> response.completed` main path, plus
/// `function_call_arguments.delta`, `custom_tool_call_input.delta`,
/// `reasoning_summary_part.added/done` and `reasoning_summary_text.delta`).
///
/// The caller performs the HTTP send (`send_stream`) and hands the peeked
/// `first_event` plus the remainder `sse_stream` to this reducer; an early
/// `error` / `response.failed` surfaces as a clean `Err` here.
///
/// # Errors
///
/// An early `error` / `response.failed` event yields an `ApiCall` error item
/// in the returned stream; malformed events yield parse-error items.
pub fn build_responses_event_stream<S>(
    first_event: Option<Result<SseEvent, SseError>>,
    sse_stream: S,
    status: u16,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<SseEvent, SseError>> + Unpin + Send + 'static,
{
    // Peek at the first SSE event to detect early errors (before any output).
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
            return Err(AiMuxError::ApiCall(ApiCallError {
                // Mid-stream provider failure arrives on a successful HTTP
                // response: keep the observed 2xx status (§2.2).
                status_code: Some(status),
                provider_code: val
                    .get("response")
                    .and_then(|r| r.get("error"))
                    .or_else(|| val.get("error"))
                    .and_then(|e| e.get("type").or_else(|| e.get("code")))
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
                message: message.to_string(),
                response_body: Some(event.data.clone()),
                ..Default::default()
            }));
        }
    }

    let stream = async_stream::stream! {
        // First part: StreamStart.
        yield Ok(StreamPart::StreamStart { warnings });

        // Ongoing tool calls keyed by output_index.
        let mut ongoing_tool_calls: HashMap<usize, OngoingToolCall> = HashMap::new();
        // Active reasoning items keyed by item_id.
        let mut active_reasoning: HashMap<String, ReasoningState> = HashMap::new();

        let mut has_function_call = false;
        let mut final_usage: Option<ResponsesUsage> = None;
        let mut final_raw_usage: Option<Value> = None;
        let mut final_service_tier: Option<String> = None;
        let mut final_reasoning_context: Option<Value> = None;
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
                                error: AiMuxError::JsonParse(e.to_string()),
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
                                    .map(std::string::ToString::to_string);
                                let model_id = resp_obj
                                    .get("model")
                                    .and_then(|v| v.as_str())
                                    .map(std::string::ToString::to_string);
                                let timestamp = resp_obj
                                    .get("created_at")
                                    .and_then(serde_json::Value::as_u64)
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
                                .and_then(serde_json::Value::as_u64)
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
                                        yield Ok(StreamPart::TextStart { id, provider_metadata: None});
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
                                            title: None,
                                            provider_metadata: None,
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
                                            title: None,
                                            provider_metadata: None,
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
                                            .map(std::string::ToString::to_string);
                                        // Carry itemId + encrypted_content in
                                        // provider_metadata (same shape as the
                                        // non-streaming path) so reasoning can
                                        // be echoed back on the next turn.
                                        let meta = reasoning_stream_metadata(
                                            &provider_key,
                                            &id,
                                            encrypted.as_deref(),
                                        );
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
                                            id: format!("{id}:0"),
                                            provider_metadata: Some(meta),
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
                            yield Ok(StreamPart::TextDelta { id, delta, provider_metadata: None});
                        }

                        // ── function_call_arguments.delta → ToolInputDelta ────────
                        "response.function_call_arguments.delta"
                        | "response.custom_tool_call_input.delta" => {
                            let output_index = parsed
                                .get("output_index")
                                .and_then(serde_json::Value::as_u64)
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
                                    provider_metadata: None,
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
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0) as usize;

                            if summary_index > 0
                                && let Some(state) = active_reasoning.get_mut(&item_id)
                            {
                                let meta = reasoning_stream_metadata(
                                    &provider_key,
                                    &item_id,
                                    state.encrypted_content.as_deref(),
                                );
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
                                        id: format!("{item_id}:{idx}"),
                                        provider_metadata: Some(meta.clone()),
                                    });
                                }
                                state
                                    .summary_parts
                                    .insert(summary_index, SummaryStatus::Active);
                                yield Ok(StreamPart::ReasoningStart {
                                    id: format!("{item_id}:{summary_index}"),
                                    provider_metadata: Some(meta),
                                });
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
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0) as usize;
                            let delta = parsed
                                .get("delta")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            yield Ok(StreamPart::ReasoningDelta {
                                id: format!("{item_id}:{summary_index}"),
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
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0) as usize;
                            if let Some(state) = active_reasoning.get_mut(&item_id) {
                                if store_flag {
                                    state
                                        .summary_parts
                                        .insert(summary_index, SummaryStatus::Concluded);
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: format!("{item_id}:{summary_index}"),
                                        provider_metadata: Some(reasoning_stream_metadata(
                                            &provider_key,
                                            &item_id,
                                            state.encrypted_content.as_deref(),
                                        )),
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
                                .and_then(serde_json::Value::as_u64)
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
                                        yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
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
                                            provider_metadata: None,
                                        });
                                        let input = Value::String(arguments);
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: call_id,
                                            tool_name: name,
                                            input,
                                            provider_executed: None,
                                            dynamic: None,
                                            thought_signature: None,
                                            invalid: None,
                                            error: None,
                                            provider_metadata: None,
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
                                            provider_metadata: None,
                                        });
                                        let input = Value::String(
                                            serde_json::to_string(input_str).expect(
                                                "serializing a custom-tool input string cannot fail",
                                            ),
                                        );
                                        yield Ok(StreamPart::ToolCall {
                                            tool_call_id: call_id,
                                            tool_name: name,
                                            input,
                                            provider_executed: None,
                                            dynamic: None,
                                            thought_signature: None,
                                            invalid: None,
                                            error: None,
                                            provider_metadata: None,
                                        });
                                    }
                                    "reasoning" => {
                                        let id = item
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if let Some(state) = active_reasoning.get_mut(&id) {
                                            let meta = reasoning_stream_metadata(
                                                &provider_key,
                                                &id,
                                                state.encrypted_content.as_deref(),
                                            );
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
                                                    id: format!("{id}:{idx}"),
                                                    provider_metadata: Some(meta.clone()),
                                                });
                                            }
                                        }
                                        active_reasoning.remove(&id);
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // ── response.output_text.annotation.added → Source ─────────
                        "response.output_text.annotation.added" => {
                            if let Some(ann) = parsed.get("annotation")
                                && ann.get("type").and_then(|v| v.as_str())
                                    == Some("url_citation")
                            {
                                yield Ok(StreamPart::Source {
                                    id: generate_source_id(),
                                    source_type: "url".to_string(),
                                    url: ann
                                        .get("url")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string),
                                    title: ann
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string),
                                    provider_metadata: None,
                                });
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
                                    resp_obj.get("usage").and_then(parse_usage);
                                // Keep the raw wire usage object for `usage.raw`
                                // (RFC-0015 P0-3) and capture the finish-time
                                // provider metadata fields carried on the
                                // terminal response object.
                                final_raw_usage = resp_obj.get("usage").cloned();
                                if let Some(st) = resp_obj
                                    .get("service_tier")
                                    .and_then(|v| v.as_str())
                                {
                                    final_service_tier = Some(st.to_string());
                                }
                                if let Some(reasoning) = resp_obj.get("reasoning")
                                    && let Some(ctx) = reasoning.get("context")
                                    && !ctx.is_null()
                                {
                                    final_reasoning_context = Some(ctx.clone());
                                }
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
                                    resp_obj.get("usage").and_then(parse_usage);
                                final_raw_usage = resp_obj.get("usage").cloned();
                                // Upstream's `response.failed` arm carries
                                // `reasoningContext` (but not `serviceTier`).
                                if let Some(reasoning) = resp_obj.get("reasoning")
                                    && let Some(ctx) = reasoning.get("context")
                                    && !ctx.is_null()
                                {
                                    final_reasoning_context = Some(ctx.clone());
                                }
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
                                        error: AiMuxError::ApiCall(ApiCallError {
                                            status_code: Some(status),
                                            provider_code: resp_obj
                                                .get("error")
                                                .and_then(|e| e.get("type").or_else(|| e.get("code")))
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string),
                                            message: message.to_string(),
                                            response_body: Some(sse_event.data.clone()),
                                            ..Default::default()
                                        }),
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
                                error: AiMuxError::ApiCall(ApiCallError {
                                    status_code: Some(status),
                                    provider_code: parsed
                                        .get("error")
                                        .and_then(|e| e.get("type").or_else(|| e.get("code")))
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string),
                                    message: message.to_string(),
                                    response_body: Some(sse_event.data.clone()),
                                    ..Default::default()
                                }),
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
                        error: AiMuxError::InvalidResponseData(e.to_string()),
                    });
                    stream_errored = true;
                    break;
                }
            }
        }

        // Build provider metadata for the Finish part: { <provider_key>: {
        // responseId, serviceTier?, reasoningContext? } }. Mirrors the TS
        // flush() providerMetadata, which carries `service_tier` and
        // `reasoning.context` from the terminal response object.
        let mut pm = json!({ "responseId": response_id.unwrap_or_default() });
        if let Some(st) = final_service_tier {
            pm["serviceTier"] = json!(st);
        }
        if let Some(ctx) = final_reasoning_context {
            pm["reasoningContext"] = ctx;
        }
        let provider_metadata = Some(json!({ provider_key: pm }));

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
                convert_responses_usage(final_usage.as_ref(), final_raw_usage)
            },
            provider_metadata,
        });
    };
    Ok(Box::pin(stream))
}
