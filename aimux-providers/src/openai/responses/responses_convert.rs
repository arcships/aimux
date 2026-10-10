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

use aimux_core::tool::RawToolCall;
use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;

use aimux_provider_utils::{TransformStreamController, Transformer, pipe_through};
use futures::{Stream, StreamExt};
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source};
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Warning,
};

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
    raw_body: &str,
    request_warnings: Vec<Warning>,
    provider_key: String,
    request_url: String,
    body: Value,
    response_headers: HashMap<String, String>,
) -> Result<GenerateResult, AiMuxError> {
    build_responses_generate_result_with_tools(
        data,
        raw_body,
        request_warnings,
        provider_key,
        request_url,
        body,
        response_headers,
        HashMap::new(),
        HashMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_generate_result_with_tools(
    data: &Value,
    raw_body: &str,
    request_warnings: Vec<Warning>,
    provider_key: String,
    request_url: String,
    body: Value,
    response_headers: HashMap<String, String>,
    tool_names: HashMap<String, String>,
    prompt_approval_tool_call_ids: HashMap<String, String>,
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
        return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
            // Provider-declared in-band failure: keep the observed 2xx
            // envelope status and the full raw body (§2.2). The request body
            // is raw here (the exchange-path redaction ran on its own copy),
            // so redact before it lands in the public error.
            status_code: Some(200),
            provider_code,
            response_body: Some(raw_body.to_string()),
            response_headers: Some(response_headers.clone()),
            ..ApiCallError::new(
                message,
                request_url,
                aimux_provider_utils::redact_error_context(body.clone()),
            )
        })));
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

    let shell_provider_executed = hosted_shell(&body);
    let mut content: Vec<GenerateContent> = Vec::new();
    let mut has_function_call = false;
    let mut logprobs = Vec::new();
    let mut hosted_tool_search_ids = VecDeque::new();

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
                            if body
                                .get("top_logprobs")
                                .and_then(Value::as_u64)
                                .is_some_and(|n| n > 0)
                                && let Some(value) = cp.get("logprobs")
                            {
                                logprobs.push(value.clone());
                            }
                            {
                                content.push(GenerateContent::Text {
                                    text,
                                    provider_metadata: Some(
                                        provider_namespace(
                                            &provider_key,
                                            text_metadata(
                                                part,
                                                cp.get("annotations").and_then(Value::as_array),
                                            ),
                                        )
                                        .expect("provider metadata must be an object"),
                                    ),
                                });
                            }
                        }
                        if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array())
                        {
                            for ann in annotations {
                                content.push(GenerateContent::Source(annotation_source(
                                    ann,
                                    &provider_key,
                                )?));
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
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: call_id,
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: Some(
                        provider_namespace(&provider_key, tool_metadata(part))
                            .expect("provider metadata must be an object"),
                    ),
                }));
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
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: call_id,
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: Some(
                        provider_namespace(&provider_key, tool_metadata(part))
                            .expect("provider metadata must be an object"),
                    ),
                }));
            }
            Some("reasoning") => {
                let summary = part.get("summary").and_then(|v| v.as_array());
                let parts: Vec<&Value> = summary.map(|s| s.iter().collect()).unwrap_or_default();
                let reasoning_metadata = Some(provider_namespace(
                    &provider_key,
                    json!({
                        "itemId": part.get("id").cloned().unwrap_or(Value::Null),
                        "reasoningEncryptedContent": part.get("encrypted_content").cloned().unwrap_or(Value::Null),
                    }),
                ).expect("provider metadata must be an object"));
                if parts.is_empty() {
                    content.push(GenerateContent::Reasoning(ReasoningOutput {
                        text: String::new(),
                        provider_metadata: reasoning_metadata.clone(),
                    }));
                } else {
                    for sp in parts {
                        let text = sp
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        content.push(GenerateContent::Reasoning(ReasoningOutput {
                            text,
                            provider_metadata: reasoning_metadata.clone(),
                        }));
                    }
                }
            }
            _ => {
                let mut part = part.clone();
                pair_tool_search_id(&mut part, &mut hosted_tool_search_ids);
                let parts = super::provider_events::provider_tool_content(
                    &part,
                    &provider_key,
                    &tool_names,
                    shell_provider_executed,
                    part.get("approval_request_id")
                        .and_then(Value::as_str)
                        .and_then(|id| prompt_approval_tool_call_ids.get(id))
                        .map(String::as_str),
                );
                has_function_call |= part["type"] == "apply_patch_call"
                    || (part["type"] == "computer_call" && !part["call_id"].is_null());
                content.extend(parts);
            }
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
    if !logprobs.is_empty() {
        pm["logprobs"] = json!(logprobs);
    }
    let provider_metadata =
        Some(provider_namespace(&provider_key, pm).expect("provider metadata must be an object"));

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
        response: Some(aimux_core::shared::ResponseInfo {
            id: response_id,
            timestamp,
            model_id: model,
            headers: Some(response_headers),
            body: Some(data.clone()),
        }),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
    })
}

// -- Streaming SSE event reducer ---------------------------------------------

/// A tool call being streamed (tracked by `output_index`).
struct OngoingToolCall {
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
) -> ProviderMetadata {
    let inner = json!({ "itemId": item_id, "reasoningEncryptedContent": encrypted_content });
    provider_namespace(provider_key, inner).expect("provider metadata must be an object")
}

fn tool_metadata(item: &Value) -> Value {
    let mut metadata = json!({ "itemId": item.get("id").cloned().unwrap_or(Value::Null) });
    for key in ["async", "namespace"] {
        if let Some(value) = item.get(key).filter(|v| !v.is_null()) {
            metadata[key] = value.clone();
        }
    }
    if let Some(caller) = item.get("caller").filter(|value| !value.is_null()) {
        metadata["caller"] = if caller["type"] == "program" {
            json!({"type": "program", "callerId": caller["caller_id"]})
        } else {
            caller.clone()
        };
    }
    metadata
}

fn text_metadata(item: &Value, annotations: Option<&Vec<Value>>) -> Value {
    let mut metadata = json!({ "itemId": item.get("id").cloned().unwrap_or(Value::Null) });
    if let Some(phase) = item.get("phase").filter(|v| !v.is_null()) {
        metadata["phase"] = phase.clone();
    }
    if let Some(annotations) = annotations.filter(|v| !v.is_empty()) {
        metadata["annotations"] = json!(annotations);
    }
    metadata
}

fn annotation_source(annotation: &Value, provider_key: &str) -> Result<Source, AiMuxError> {
    let invalid = |field: &str| {
        AiMuxError::InvalidResponseData(format!("Invalid Responses annotation field: {field}"))
    };
    let string = |field| {
        annotation
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(field))
    };
    let number = |field| {
        annotation
            .get(field)
            .filter(|value| value.is_number())
            .cloned()
            .ok_or_else(|| invalid(field))
    };
    let annotation_type = string("type")?;
    if annotation_type == "url_citation" {
        number("start_index")?;
        number("end_index")?;
        return Ok(Source::Url {
            id: generate_source_id(),
            url: string("url")?.to_string(),
            title: Some(string("title")?.to_string()),
            provider_metadata: None,
        });
    }
    let file_id = string("file_id")?;
    let (media_type, filename, metadata) = match annotation_type {
        "file_citation" | "file_path" => (
            if annotation_type == "file_path" {
                "application/octet-stream"
            } else {
                "text/plain"
            },
            if annotation_type == "file_path" {
                file_id
            } else {
                string("filename")?
            },
            json!({ "type": annotation_type, "fileId": file_id, "index": number("index")? }),
        ),
        "container_file_citation" => {
            number("start_index")?;
            number("end_index")?;
            (
                "text/plain",
                string("filename")?,
                json!({
                    "type": annotation_type,
                    "fileId": file_id,
                    "containerId": string("container_id")?,
                }),
            )
        }
        _ => return Err(invalid("type")),
    };
    Ok(Source::Document {
        id: generate_source_id(),
        media_type: media_type.to_string(),
        title: filename.to_string(),
        filename: Some(filename.to_string()),
        provider_metadata: Some(provider_namespace(provider_key, metadata)?),
    })
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
/// The caller performs the API call and hands the peeked
/// `first_event` plus the remainder `sse_stream` to this reducer; an early
/// `error` / `response.failed` surfaces as a clean `Err` here.
///
/// # Errors
///
/// An early `error` / `response.failed` event returns an `ApiCall` setup error;
/// malformed events yield parse-error items and do not end the stream.
// This reducer is shared by OpenAI, Azure, and Codex. Keeping the wire inputs
// explicit is clearer than introducing a second context object used nowhere
// else.
#[allow(clippy::too_many_arguments)]
pub fn build_responses_event_stream<S>(
    first_event: Option<Result<Value, AiMuxError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin + Send + 'static,
{
    build_responses_event_stream_with_raw(
        first_event,
        sse_stream,
        provider_key,
        warnings,
        store_flag,
        request_url,
        request_body,
        response_headers,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_event_stream_with_raw<S>(
    first_event: Option<Result<Value, AiMuxError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
    include_raw_chunks: bool,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin + Send + 'static,
{
    build_responses_event_stream_with_tools(
        first_event,
        sse_stream,
        provider_key,
        warnings,
        store_flag,
        request_url,
        request_body,
        response_headers,
        include_raw_chunks,
        HashMap::new(),
        HashMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_event_stream_with_tools<S>(
    first_event: Option<Result<Value, AiMuxError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
    include_raw_chunks: bool,
    tool_names: HashMap<String, String>,
    prompt_approval_tool_call_ids: HashMap<String, String>,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin + Send + 'static,
{
    // Peek at the first SSE event to detect early errors (before any output).
    if let Some(Ok(ref val)) = first_event {
        let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if etype == "error" || etype == "response.failed" {
            return Err(super::super::openai_stream_error(
                val,
                &request_url,
                request_body.clone(),
                response_headers.clone(),
            ));
        }
    }

    let shell_provider_executed = hosted_shell(&request_body);
    let stream = pipe_through(
        futures::stream::iter(first_event).chain(sse_stream),
        OpenAIResponsesStream {
            warnings,
            provider_key,
            store_flag,
            request_url,
            request_body,
            response_headers,
            include_raw_chunks,
            tool_names,
            prompt_approval_tool_call_ids,
            shell_provider_executed,
            ongoing_tool_calls: HashMap::new(),
            active_reasoning: HashMap::new(),
            has_function_call: false,
            final_usage: None,
            final_raw_usage: None,
            final_service_tier: None,
            final_reasoning_context: None,
            final_finish_reason: None,
            response_id: None,
            stream_errored: false,
            text_states: HashMap::new(),
            output_ids: HashMap::new(),
            text_annotations: HashMap::new(),
            logprobs: Vec::new(),
            provider_items: HashMap::new(),
            emitted_calls: HashSet::new(),
            ended_inputs: HashSet::new(),
            diff_received: HashSet::new(),
            hosted_tool_search_ids: VecDeque::new(),
            approval_tool_call_ids: HashMap::new(),
        },
    );
    Ok(Box::pin(stream))
}

/// The `TransformStream` of the OpenAI Responses `doStream`.
struct OpenAIResponsesStream {
    warnings: Vec<Warning>,
    provider_key: String,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
    include_raw_chunks: bool,
    tool_names: HashMap<String, String>,
    prompt_approval_tool_call_ids: HashMap<String, String>,
    shell_provider_executed: bool,
    /// Ongoing tool calls keyed by output_index.
    ongoing_tool_calls: HashMap<usize, OngoingToolCall>,
    /// Active reasoning items keyed by item_id.
    active_reasoning: HashMap<String, ReasoningState>,
    has_function_call: bool,
    final_usage: Option<ResponsesUsage>,
    final_raw_usage: Option<Value>,
    final_service_tier: Option<String>,
    final_reasoning_context: Option<Value>,
    final_finish_reason: Option<FinishReason>,
    response_id: Option<String>,
    stream_errored: bool,
    text_states: HashMap<String, Value>,
    output_ids: HashMap<usize, String>,
    text_annotations: HashMap<String, Vec<Value>>,
    logprobs: Vec<Value>,
    provider_items: HashMap<usize, Value>,
    emitted_calls: HashSet<String>,
    ended_inputs: HashSet<String>,
    diff_received: HashSet<String>,
    hosted_tool_search_ids: VecDeque<Value>,
    approval_tool_call_ids: HashMap<String, String>,
}

impl Transformer for OpenAIResponsesStream {
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
        let Self {
            ref provider_key,
            store_flag,
            ref request_url,
            ref request_body,
            ref response_headers,
            include_raw_chunks,
            ref tool_names,
            ref prompt_approval_tool_call_ids,
            shell_provider_executed,
            ref mut ongoing_tool_calls,
            ref mut active_reasoning,
            ref mut has_function_call,
            ref mut final_usage,
            ref mut final_raw_usage,
            ref mut final_service_tier,
            ref mut final_reasoning_context,
            ref mut final_finish_reason,
            ref mut response_id,
            ref mut stream_errored,
            ref mut text_states,
            ref mut output_ids,
            ref mut text_annotations,
            ref mut logprobs,
            ref mut provider_items,
            ref mut emitted_calls,
            ref mut ended_inputs,
            ref mut diff_received,
            ref mut hosted_tool_search_ids,
            ref mut approval_tool_call_ids,
            ..
        } = *self;
        match event {
            Ok(parsed) => {
                if include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: parsed.clone(),
                    });
                }
                let etype = parsed
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                match etype.as_str() {
                    // ── response.created → ResponseMetadata ───────────────────
                    "response.created" => {
                        if let Some(resp_obj) = parsed.get("response") {
                            *response_id = resp_obj
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
                                .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                                .map(|dt| dt.to_rfc3339());
                            controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                                id: response_id.clone(),
                                timestamp,
                                model_id,
                            }));
                        }
                    }

                    // ── response.output_item.added ─────────────────────────────
                    "response.output_item.added" => {
                        let output_index = parsed
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        if let Some(item) = parsed.get("item") {
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match item_type {
                                "message" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    output_ids.insert(output_index, id.clone());
                                    text_states.insert(id.clone(), item.clone());
                                    controller.enqueue(StreamPart::TextStart {
                                        id,
                                        provider_metadata: Some(
                                            provider_namespace(
                                                provider_key,
                                                text_metadata(item, None),
                                            )
                                            .expect("provider metadata must be an object"),
                                        ),
                                    });
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
                                            tool_call_id: call_id.clone(),
                                        },
                                    );
                                    controller.enqueue(StreamPart::ToolInputStart {
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
                                            tool_call_id: call_id.clone(),
                                        },
                                    );
                                    controller.enqueue(StreamPart::ToolInputStart {
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
                                        provider_key,
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
                                    controller.enqueue(StreamPart::ReasoningStart {
                                        id: format!("{id}:0"),
                                        provider_metadata: Some(meta),
                                    });
                                }
                                _ => {
                                    provider_items.insert(output_index, item.clone());
                                    for part in provider_tool_start(item, tool_names) {
                                        if let StreamPart::ToolCall(call) = &part {
                                            emitted_calls.insert(call.tool_call_id.clone());
                                        }
                                        if let StreamPart::ToolInputEnd { id, .. } = &part {
                                            ended_inputs.insert(id.clone());
                                        }
                                        controller.enqueue(part);
                                    }
                                }
                            }
                        }
                    }

                    "response.image_generation_call.partial_image" => {
                        controller.enqueue(StreamPart::ToolResult(aimux_core::tool::ToolResult {
                            tool_call_id: parsed["item_id"].as_str().unwrap_or_default().to_owned(),
                            tool_name: tool_names
                                .get("image_generation")
                                .map_or("image_generation", String::as_str)
                                .to_owned(),
                            result: json!({"result": parsed["partial_image_b64"]}),
                            preliminary: Some(true),
                            is_error: None,
                            dynamic: None,
                            provider_metadata: None,
                        }));
                    }
                    "response.code_interpreter_call_code.delta"
                    | "response.apply_patch_call_operation_diff.delta" => {
                        let index = parsed["output_index"].as_u64().unwrap_or(0) as usize;
                        if let Some(item) = provider_items.get(&index) {
                            let id = item
                                .get("call_id")
                                .filter(|v| !v.is_null())
                                .unwrap_or(&item["id"])
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            diff_received.insert(id.clone());
                            controller.enqueue(StreamPart::ToolInputDelta {
                                id,
                                delta: escaped_delta(parsed["delta"].as_str().unwrap_or_default()),
                                provider_metadata: None,
                            });
                        }
                    }
                    "response.code_interpreter_call_code.done"
                    | "response.apply_patch_call_operation_diff.done" => {
                        let index = parsed["output_index"].as_u64().unwrap_or(0) as usize;
                        if let Some(item) = provider_items.get(&index) {
                            let id = item
                                .get("call_id")
                                .filter(|v| !v.is_null())
                                .unwrap_or(&item["id"])
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            if ended_inputs.insert(id.clone()) {
                                let code = etype == "response.code_interpreter_call_code.done";
                                if !code && !diff_received.contains(&id) {
                                    controller.enqueue(StreamPart::ToolInputDelta {
                                        id: id.clone(),
                                        delta: escaped_delta(
                                            parsed["diff"].as_str().unwrap_or_default(),
                                        ),
                                        provider_metadata: None,
                                    });
                                }
                                controller.enqueue(StreamPart::ToolInputDelta {
                                    id: id.clone(),
                                    delta: if code { "\"}" } else { "\"}}" }.to_owned(),
                                    provider_metadata: None,
                                });
                                controller.enqueue(StreamPart::ToolInputEnd {
                                    id: id.clone(),
                                    provider_metadata: None,
                                });
                                if code {
                                    let mut item = item.clone();
                                    item["code"] = parsed["code"].clone();
                                    for part in super::provider_events::provider_tool_content(
                                        &item,
                                        provider_key,
                                        tool_names,
                                        shell_provider_executed,
                                        None,
                                    ) {
                                        if let GenerateContent::ToolCall(call) = part {
                                            emitted_calls.insert(id.clone());
                                            controller.enqueue(StreamPart::ToolCall(call));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── response.output_text.delta → TextDelta ────────────────
                    "response.output_text.delta" => {
                        if request_body
                            .get("top_logprobs")
                            .and_then(Value::as_u64)
                            .is_some_and(|n| n > 0)
                            && let Some(value) = parsed.get("logprobs")
                        {
                            logprobs.push(value.clone());
                        }
                        let mut id = parsed
                            .get("item_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(index) = parsed.get("output_index").and_then(Value::as_u64)
                            && let Some(original_id) = output_ids.get(&(index as usize))
                        {
                            id = original_id.clone();
                        }
                        let delta = parsed
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        controller.enqueue(StreamPart::TextDelta {
                            id,
                            delta,
                            provider_metadata: None,
                        });
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
                            controller.enqueue(StreamPart::ToolInputDelta {
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
                                provider_key,
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
                                controller.enqueue(StreamPart::ReasoningEnd {
                                    id: format!("{item_id}:{idx}"),
                                    provider_metadata: Some(meta.clone()),
                                });
                            }
                            state
                                .summary_parts
                                .insert(summary_index, SummaryStatus::Active);
                            controller.enqueue(StreamPart::ReasoningStart {
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
                        controller.enqueue(StreamPart::ReasoningDelta {
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
                                controller.enqueue(StreamPart::ReasoningEnd {
                                    id: format!("{item_id}:{summary_index}"),
                                    provider_metadata: Some(reasoning_stream_metadata(
                                        provider_key,
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
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match item_type {
                                "message" => {
                                    let mut id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if let Some(original_id) = output_ids.remove(&output_index) {
                                        id = original_id;
                                    }
                                    let mut metadata_item = item.clone();
                                    metadata_item["id"] = json!(id);
                                    if metadata_item.get("phase").is_none()
                                        && let Some(phase) =
                                            text_states.get(&id).and_then(|v| v.get("phase"))
                                    {
                                        metadata_item["phase"] = phase.clone();
                                    }
                                    let annotations = text_annotations.remove(&id);
                                    controller.enqueue(StreamPart::TextEnd {
                                        id,
                                        provider_metadata: Some(
                                            provider_namespace(
                                                provider_key,
                                                text_metadata(&metadata_item, annotations.as_ref()),
                                            )
                                            .expect("provider metadata must be an object"),
                                        ),
                                    });
                                }
                                "function_call" => {
                                    *has_function_call = true;
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
                                    controller.enqueue(StreamPart::ToolInputEnd {
                                        id: call_id.clone(),
                                        provider_metadata: None,
                                    });
                                    let input = arguments;
                                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: call_id,
                                        tool_name: name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        provider_metadata: Some(
                                            provider_namespace(provider_key, tool_metadata(item))
                                                .expect("provider metadata must be an object"),
                                        ),
                                    }));
                                }
                                "custom_tool_call" => {
                                    *has_function_call = true;
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
                                    let input_str =
                                        item.get("input").and_then(|v| v.as_str()).unwrap_or("{}");
                                    controller.enqueue(StreamPart::ToolInputEnd {
                                        id: call_id.clone(),
                                        provider_metadata: None,
                                    });
                                    let input = serde_json::to_string(input_str).expect(
                                        "serializing a custom-tool input string cannot fail",
                                    );
                                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: call_id,
                                        tool_name: name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        provider_metadata: Some(
                                            provider_namespace(provider_key, tool_metadata(item))
                                                .expect("provider metadata must be an object"),
                                        ),
                                    }));
                                }
                                "reasoning" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if let Some(state) = active_reasoning.get_mut(&id) {
                                        state.encrypted_content = item
                                            .get("encrypted_content")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned);
                                        let meta = reasoning_stream_metadata(
                                            provider_key,
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
                                            controller.enqueue(StreamPart::ReasoningEnd {
                                                id: format!("{id}:{idx}"),
                                                provider_metadata: Some(meta.clone()),
                                            });
                                        }
                                    }
                                    active_reasoning.remove(&id);
                                }
                                _ => {
                                    let mut item = item.clone();
                                    if item_type == "local_shell_call"
                                        && let Some(action) = item["action"].as_object_mut()
                                    {
                                        for (from, to) in [
                                            ("timeout_ms", "timeoutMs"),
                                            ("working_directory", "workingDirectory"),
                                        ] {
                                            if let Some(value) = action.remove(from) {
                                                action.insert(to.into(), value);
                                            }
                                        }
                                    }
                                    if item_type == "apply_patch_call" {
                                        let id =
                                            item["call_id"].as_str().unwrap_or_default().to_owned();
                                        if ended_inputs.insert(id.clone())
                                            && item["operation"]["type"] != "delete_file"
                                        {
                                            if !diff_received.contains(&id) {
                                                controller.enqueue(StreamPart::ToolInputDelta {
                                                    id: id.clone(),
                                                    delta: escaped_delta(
                                                        item["operation"]["diff"]
                                                            .as_str()
                                                            .unwrap_or_default(),
                                                    ),
                                                    provider_metadata: None,
                                                });
                                            }
                                            controller.enqueue(StreamPart::ToolInputDelta {
                                                id: id.clone(),
                                                delta: "\"}}".into(),
                                                provider_metadata: None,
                                            });
                                            controller.enqueue(StreamPart::ToolInputEnd {
                                                id,
                                                provider_metadata: None,
                                            });
                                        }
                                    }
                                    if item_type == "tool_search_call"
                                        && item["execution"] == "server"
                                    {
                                        item["call_id"] = Value::Null;
                                    }
                                    pair_tool_search_id(&mut item, hosted_tool_search_ids);
                                    let mcp_tool_call_id = item
                                        .get("approval_request_id")
                                        .and_then(Value::as_str)
                                        .and_then(|id| {
                                            approval_tool_call_ids
                                                .get(id)
                                                .or_else(|| prompt_approval_tool_call_ids.get(id))
                                        })
                                        .cloned();
                                    for part in super::provider_events::provider_tool_content(
                                        &item,
                                        provider_key,
                                        tool_names,
                                        shell_provider_executed,
                                        mcp_tool_call_id.as_deref(),
                                    ) {
                                        match part {
                                            GenerateContent::ToolCall(call) => {
                                                if item_type == "code_interpreter_call"
                                                    || (item_type == "apply_patch_call"
                                                        && item["status"] != "completed")
                                                {
                                                    continue;
                                                }
                                                if emitted_calls.insert(call.tool_call_id.clone())
                                                    || item_type == "mcp_call"
                                                {
                                                    *has_function_call |= call.provider_executed
                                                        != Some(true)
                                                        && matches!(
                                                            item_type,
                                                            "computer_call" | "apply_patch_call"
                                                        );
                                                    if matches!(
                                                        item_type,
                                                        "computer_call"
                                                            | "apply_patch_call"
                                                            | "tool_search_call"
                                                    ) {
                                                        if item_type == "tool_search_call"
                                                            && item["execution"] != "server"
                                                        {
                                                            controller.enqueue(
                                                                StreamPart::ToolInputStart {
                                                                    id: call.tool_call_id.clone(),
                                                                    tool_name: call
                                                                        .tool_name
                                                                        .clone(),
                                                                    provider_executed: call
                                                                        .provider_executed,
                                                                    dynamic: None,
                                                                    title: None,
                                                                    provider_metadata: None,
                                                                },
                                                            );
                                                        }
                                                        if !ended_inputs
                                                            .contains(&call.tool_call_id)
                                                        {
                                                            if item_type == "computer_call"
                                                                && !call.input.is_empty()
                                                            {
                                                                controller.enqueue(
                                                                    StreamPart::ToolInputDelta {
                                                                        id: call
                                                                            .tool_call_id
                                                                            .clone(),
                                                                        delta: call.input.clone(),
                                                                        provider_metadata: None,
                                                                    },
                                                                );
                                                            }
                                                            controller.enqueue(
                                                                StreamPart::ToolInputEnd {
                                                                    id: call.tool_call_id.clone(),
                                                                    provider_metadata: None,
                                                                },
                                                            );
                                                        }
                                                    }
                                                    controller.enqueue(StreamPart::ToolCall(call));
                                                }
                                            }
                                            GenerateContent::ToolResult(result) => {
                                                controller.enqueue(StreamPart::ToolResult(result));
                                            }
                                            GenerateContent::ToolApprovalRequest(approval) => {
                                                approval_tool_call_ids.insert(
                                                    approval.approval_id.clone(),
                                                    approval.tool_call_id.clone(),
                                                );
                                                controller.enqueue(
                                                    StreamPart::ToolApprovalRequest(approval),
                                                );
                                            }
                                            GenerateContent::Custom {
                                                kind,
                                                provider_metadata,
                                            } => {
                                                controller.enqueue(StreamPart::Custom {
                                                    kind,
                                                    provider_metadata,
                                                });
                                            }
                                            _ => {}
                                        }
                                    }
                                    provider_items.remove(&output_index);
                                }
                            }
                        }
                    }

                    // ── response.output_text.annotation.added → Source ─────────
                    "response.output_text.annotation.added" => {
                        match annotation_source(
                            parsed.get("annotation").unwrap_or(&Value::Null),
                            provider_key,
                        ) {
                            Ok(source) => {
                                let id = parsed
                                    .get("item_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                text_annotations
                                    .entry(id)
                                    .or_default()
                                    .push(parsed["annotation"].clone());
                                controller.enqueue(StreamPart::Source(source));
                            }
                            Err(error) => {
                                *stream_errored = true;
                                controller.enqueue(StreamPart::Error { error });
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
                            if !*stream_errored {
                                *final_finish_reason =
                                    Some(map_responses_finish_reason(reason, *has_function_call));
                            }
                            *final_usage = resp_obj.get("usage").and_then(parse_usage);
                            // Keep the raw wire usage object for `usage.raw`
                            // (RFC-0015 P0-3) and capture the finish-time
                            // provider metadata fields carried on the
                            // terminal response object.
                            *final_raw_usage = resp_obj.get("usage").cloned();
                            if let Some(st) = resp_obj.get("service_tier").and_then(|v| v.as_str())
                            {
                                *final_service_tier = Some(st.to_string());
                            }
                            if let Some(reasoning) = resp_obj.get("reasoning")
                                && let Some(ctx) = reasoning.get("context")
                                && !ctx.is_null()
                            {
                                *final_reasoning_context = Some(ctx.clone());
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
                            *final_finish_reason = Some(match reason {
                                Some(r) => map_responses_finish_reason(Some(r), *has_function_call),
                                None => FinishReason {
                                    unified: FinishReasonUnified::Error,
                                    raw: Some("error".to_string()),
                                },
                            });
                            *final_usage = resp_obj.get("usage").and_then(parse_usage);
                            *final_raw_usage = resp_obj.get("usage").cloned();
                            // Upstream's `response.failed` arm carries
                            // `reasoningContext` (but not `serviceTier`).
                            if let Some(reasoning) = resp_obj.get("reasoning")
                                && let Some(ctx) = reasoning.get("context")
                                && !ctx.is_null()
                            {
                                *final_reasoning_context = Some(ctx.clone());
                            }
                            if !*stream_errored && resp_obj.get("error").is_some() {
                                *stream_errored = true;
                                controller.enqueue(StreamPart::Error {
                                    error: super::super::openai_stream_error(
                                        &parsed,
                                        request_url,
                                        request_body.clone(),
                                        response_headers.clone(),
                                    ),
                                });
                            }
                        }
                    }

                    // ── error chunk ───────────────────────────────────────────
                    "error" => {
                        *stream_errored = true;
                        *final_finish_reason = Some(FinishReason {
                            unified: FinishReasonUnified::Error,
                            raw: Some("error".to_string()),
                        });
                        controller.enqueue(StreamPart::Error {
                            error: super::super::openai_stream_error(
                                &parsed,
                                request_url,
                                request_body.clone(),
                                response_headers.clone(),
                            ),
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
            Err(error) => {
                *stream_errored = true;
                *final_finish_reason = Some(FinishReason {
                    unified: FinishReasonUnified::Error,
                    raw: None,
                });
                controller.enqueue(StreamPart::Error { error });
            }
        }
    }

    fn flush(self, controller: &mut TransformStreamController<StreamPart>) {
        // Build provider metadata for the Finish part: { <provider_key>: {
        // responseId, serviceTier?, reasoningContext? } }. Mirrors the TS
        // flush() providerMetadata, which carries `service_tier` and
        // `reasoning.context` from the terminal response object.
        let mut pm = json!({ "responseId": self.response_id.unwrap_or_default() });
        if let Some(st) = self.final_service_tier {
            pm["serviceTier"] = json!(st);
        }
        if let Some(ctx) = self.final_reasoning_context {
            pm["reasoningContext"] = ctx;
        }
        if !self.logprobs.is_empty() {
            pm["logprobs"] = json!(self.logprobs);
        }
        let provider_metadata = Some(
            provider_namespace(&self.provider_key, pm)
                .expect("provider metadata must be an object"),
        );

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            }),
            usage: convert_responses_usage(self.final_usage.as_ref(), self.final_raw_usage),
            provider_metadata,
        });
    }
}

fn pair_tool_search_id(item: &mut Value, hosted_ids: &mut VecDeque<Value>) {
    match item["type"].as_str() {
        Some("tool_search_call") if item["execution"] == "server" => {
            hosted_ids.push_back(
                item.get("call_id")
                    .filter(|v| !v.is_null())
                    .unwrap_or(&item["id"])
                    .clone(),
            );
        }
        Some("tool_search_output") if item.get("call_id").is_none_or(Value::is_null) => {
            if let Some(id) = hosted_ids.pop_front() {
                item["call_id"] = id;
            }
        }
        _ => {}
    }
}

fn escaped_delta(value: &str) -> String {
    let value = json!(value).to_string();
    value[1..value.len() - 1].to_owned()
}

fn provider_tool_start(item: &Value, names: &HashMap<String, String>) -> Vec<StreamPart> {
    let kind = item["type"].as_str().unwrap_or_default();
    let canonical = match kind {
        "web_search_call" => "web_search",
        "file_search_call" => "file_search",
        "image_generation_call" => "image_generation",
        "code_interpreter_call" => "code_interpreter",
        "computer_call" => "computer",
        "apply_patch_call" => "apply_patch",
        "tool_search_call" if item["execution"] == "server" => "tool_search",
        _ => return Vec::new(),
    };
    let name = names
        .get(canonical)
        .map_or(canonical, String::as_str)
        .to_owned();
    let id = item
        .get("call_id")
        .filter(|v| !v.is_null())
        .unwrap_or(&item["id"])
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let hosted = matches!(
        kind,
        "web_search_call"
            | "file_search_call"
            | "image_generation_call"
            | "code_interpreter_call"
            | "tool_search_call"
    )
    .then_some(true);
    let call = || {
        StreamPart::ToolCall(RawToolCall {
            tool_call_id: id.clone(),
            tool_name: name.clone(),
            input: "{}".to_owned(),
            provider_executed: hosted,
            dynamic: None,
            provider_metadata: None,
        })
    };
    if matches!(kind, "file_search_call" | "image_generation_call") {
        return vec![call()];
    }
    let mut parts = vec![StreamPart::ToolInputStart {
        id: id.clone(),
        tool_name: name.clone(),
        provider_executed: hosted,
        dynamic: None,
        title: None,
        provider_metadata: None,
    }];
    match kind {
        "web_search_call" => {
            parts.push(StreamPart::ToolInputEnd {
                id: id.clone(),
                provider_metadata: None,
            });
            parts.push(call());
        }
        "code_interpreter_call" => parts.push(StreamPart::ToolInputDelta {
            id,
            delta: format!("{{\"containerId\":{},\"code\":\"", item["container_id"]),
            provider_metadata: None,
        }),
        "apply_patch_call" => {
            let operation = &item["operation"];
            if operation["type"] == "delete_file" {
                parts.push(StreamPart::ToolInputDelta {
                    id: id.clone(),
                    delta: json!({"callId":id,"operation":operation}).to_string(),
                    provider_metadata: None,
                });
                parts.push(StreamPart::ToolInputEnd {
                    id,
                    provider_metadata: None,
                });
            } else {
                parts.push(StreamPart::ToolInputDelta {
                    id: id.clone(),
                    delta: format!(
                        "{{\"callId\":{},\"operation\":{{\"type\":{},\"path\":{},\"diff\":\"",
                        json!(id),
                        operation["type"],
                        operation["path"]
                    ),
                    provider_metadata: None,
                });
            }
        }
        _ => {}
    }
    parts
}

fn hosted_shell(body: &Value) -> bool {
    body["tools"].as_array().is_some_and(|tools| {
        tools.iter().any(|tool| {
            tool["type"] == "shell"
                && tool.get("environment").is_some_and(|environment| {
                    !matches!(environment["type"].as_str(), Some("local"))
                })
        })
    })
}
