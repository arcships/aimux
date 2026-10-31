//! The Open Responses language model (responses/open-responses-language-model.ts).

use std::collections::HashMap;

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::RawToolCall;
use aimux_core::tool::Tool;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ReasoningEffort, Usage, Warning,
};
use aimux_provider_utils::{Transformer, pipe_through};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use crate::shared::EndpointConfig;
use crate::stream::{Controller, Emit, FINISH_OTHER};

use super::convert_to_input::{
    convert_to_open_responses_input_with_namespace, url_citations, validate_file_media,
};
use super::map_finish_reason::map_open_responses_finish_reason;

fn open_responses_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error");
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Open Responses request failed")
                .to_string(),
            provider_code: error
                .and_then(|value| value.get("code").or_else(|| value.get("type")))
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    })
}

fn open_responses_successful_response_handler() -> aimux_provider_utils::ResponseHandler<Value> {
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let url = input.url.clone();
        let request_body_values = input.request_body_values.clone();
        let output = aimux_provider_utils::create_json_response_handler::<Value>()
            .handle(input)
            .await?;
        if let Some(error) = output
            .value
            .get("error")
            .and_then(Value::as_object)
            .filter(|error| !error.is_empty())
        {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Open Responses request failed");
            // `errorMetadata?.statusCode ?? 400`: the HTTP status is a 200.
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(400),
                data: Some(Value::Object(error.clone())),
                provider_code: error
                    .get("code")
                    .or_else(|| error.get("type"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                response_body: Some(output.value.to_string()),
                response_headers: Some(output.response_headers.clone()),
                ..ApiCallError::new(message, url, request_body_values)
            })));
        }
        // A response without `output` is an APICallError with status 500
        // (open-responses-language-model.ts:428-441). It is built here so it
        // carries the HTTP layer's sanitized URL and request values.
        if output.value.get("output").is_none_or(Value::is_null) {
            let detail = output
                .value
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(Value::as_str)
                .or_else(|| output.value.get("status").and_then(Value::as_str));
            let message = match detail {
                Some(d) => format!("Responses API returned no output ({d})"),
                None => "Responses API returned no output".to_string(),
            };
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(500),
                response_body: Some(output.value.to_string()),
                response_headers: Some(output.response_headers.clone()),
                is_retryable: false,
                ..ApiCallError::new(message, url, request_body_values)
            })));
        }
        Ok(output)
    })
}

// == Model ==

/// An Open Responses language model.
pub struct OpenResponsesModel {
    pub(crate) model_id: String,
    /// The providerOptions key read (first segment of the provider name).
    pub(crate) provider_options_name: String,
    pub(crate) config: EndpointConfig,
}

#[async_trait]
impl LanguageModel for OpenResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Image URLs go to the API as they are
    /// (open-responses-language-model.ts:101-103).
    async fn supported_urls(&self) -> SupportedUrls {
        let http = regex::Regex::new(r"^https?://.*$").expect("static pattern");
        SupportedUrls(std::iter::once(("image/*".to_string(), vec![http])).collect())
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        validate_file_media(&options.prompt)?;
        let (body, warnings) =
            build_request_body(&self.model_id, options, &self.provider_options_name);
        let body = exchange.transform_body(body);

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/responses"), options),
            body.clone(),
            open_responses_successful_response_handler(),
            open_responses_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw: Value = resp.value;

        // The response handler has already failed a missing `output`.
        let output = raw.get("output");

        // Build content array from output items.
        let mut content = Vec::new();
        let mut has_tool_calls = false;

        if let Some(output_arr) = output.and_then(|o| o.as_array()) {
            for part in output_arr {
                let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match part_type {
                    "reasoning" => match part
                        .get("content")
                        .and_then(Value::as_array)
                        .filter(|parts| !parts.is_empty())
                    {
                        Some(content_parts) => {
                            for cp in content_parts {
                                if let Some(text) = cp.get("text").and_then(|t| t.as_str()) {
                                    content.push(GenerateContent::Reasoning(ReasoningOutput {
                                        text: text.to_string(),
                                        provider_metadata: reasoning_metadata(
                                            &self.provider_options_name,
                                            part,
                                            &json!([cp]),
                                        ),
                                    }));
                                }
                            }
                        }
                        // No reasoning content (a summary or encrypted-only
                        // item): one part with the joined summary text.
                        None => {
                            let text = part["summary"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|summary| summary["text"].as_str())
                                .collect::<String>();
                            content.push(GenerateContent::Reasoning(ReasoningOutput {
                                text,
                                provider_metadata: reasoning_metadata(
                                    &self.provider_options_name,
                                    part,
                                    &part["content"],
                                ),
                            }));
                        }
                    },
                    "message" => {
                        if let Some(content_parts) = part.get("content").and_then(|c| c.as_array())
                        {
                            for cp in content_parts {
                                if let Some(text) = cp.get("text").and_then(|t| t.as_str()) {
                                    content.push(GenerateContent::Text {
                                        text: text.to_string(),
                                        provider_metadata: message_metadata(
                                            &self.provider_options_name,
                                            part,
                                            output_text_annotations(std::slice::from_ref(cp)),
                                        ),
                                    });
                                }
                            }
                        }
                    }
                    "function_call" => {
                        has_tool_calls = true;
                        let call_id = part
                            .get("call_id")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = part
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arguments = part
                            .get("arguments")
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");
                        let input = arguments.to_string();
                        content.push(GenerateContent::ToolCall(RawToolCall {
                            tool_call_id: call_id,
                            tool_name: name,
                            input,
                            provider_executed: None,
                            dynamic: None,
                            provider_metadata: item_id_metadata(&self.provider_options_name, part),
                        }));
                    }
                    _ => {}
                }
            }
        }

        let usage = extract_usage(&raw);
        let incomplete_reason = raw
            .get("incomplete_details")
            .and_then(|d| d.get("reason"))
            .and_then(|r| r.as_str());

        let finish_reason = FinishReason {
            unified: map_open_responses_finish_reason(incomplete_reason, has_tool_calls),
            raw: incomplete_reason.map(std::string::ToString::to_string),
        };

        let response = aimux_provider_utils::create_language_model_response_metadata(
            raw.get("id")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            raw.get("model")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            raw.get("created_at").and_then(serde_json::Value::as_f64),
        );

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: None,
            response: Some(aimux_core::shared::ResponseInfo {
                headers: Some(response_headers),
                body: Some(raw),
                ..response.into()
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        validate_file_media(&options.prompt)?;
        let (body, warnings) =
            build_request_body(&self.model_id, options, &self.provider_options_name);

        let stream_body = {
            let mut b = body.clone();
            if let Some(obj) = b.as_object_mut() {
                obj.insert("stream".to_string(), json!(true));
            }
            exchange.transform_body(b)
        };
        let endpoint = exchange.url("/responses");

        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            stream_body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            open_responses_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;

        // Peek at the first SSE event to detect early errors.
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        if let Some(Ok(ref event)) = first_event
            && let Some(err_obj) = event.get("error")
        {
            let message = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("stream error");
            let status_code = err_obj
                .get("status")
                .or_else(|| err_obj.get("code"))
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok())
                .filter(|status| (400..=599).contains(status));
            let provider_code = err_obj
                .get("code")
                .or_else(|| err_obj.get("type"))
                .and_then(|c| c.as_str())
                .map(std::string::ToString::to_string);
            return Err(aimux_provider_utils::create_provider_stream_error(
                aimux_provider_utils::ProviderStreamError {
                    message: message.into(),
                    provider_code,
                    status_code,
                    is_retryable: None,
                    data: event.clone(),
                },
                event,
                endpoint,
                stream_body,
                response_headers.clone(),
            ));
        }

        let stream = pipe_through(
            futures::stream::iter(first_event).chain(sse_stream),
            OpenResponsesStream {
                provider_options_name: self.provider_options_name.clone(),
                url: endpoint,
                request_body: stream_body,
                response_headers: response_headers.clone(),
                warnings,
                final_usage: Usage::default(),
                has_tool_calls: false,
                finish_reason: FINISH_OTHER,
                active_reasoning_id: None,
                tool_calls: HashMap::new(),
                include_raw_chunks: options.include_raw_chunks == Some(true),
                strict: options.strict_stream == Some(true),
                terminal_seen: false,
            },
        );

        Ok(crate::stream::strict_guard(
            StreamResult {
                stream: Box::pin(stream),
                request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
                response: Some(aimux_core::shared::StreamResponseInfo {
                    headers: Some(response_headers),
                }),
            },
            options,
        ))
    }
}

/// The `TransformStream` of `OpenResponsesLanguageModel.doStream`.
struct OpenResponsesStream {
    provider_options_name: String,
    /// The request, for the errors of in-stream error events.
    url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
    warnings: Vec<Warning>,
    final_usage: Usage,
    has_tool_calls: bool,
    finish_reason: FinishReason,
    /// The reasoning item started and not yet done.
    active_reasoning_id: Option<String>,
    /// Tool-call accumulators keyed by item_id.
    tool_calls: HashMap<String, ToolCallAccum>,
    include_raw_chunks: bool,
    /// Strict stream mode (`CallOptions.strict_stream`).
    strict: bool,
    /// A `response.completed`, `response.incomplete`, `response.failed` or
    /// `error` event arrived.
    terminal_seen: bool,
}

impl Transformer for OpenResponsesStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        controller.enqueue(StreamPart::StreamStart {
            warnings: std::mem::take(&mut self.warnings),
        });
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        if matches!(
            event
                .as_ref()
                .ok()
                .and_then(|event| event.get("type"))
                .and_then(Value::as_str),
            Some("response.completed" | "response.incomplete" | "response.failed" | "error")
        ) {
            self.terminal_seen = true;
        }
        // Every chunk first, as `raw` when requested
        // (open-responses-language-model.ts:685-690); a chunk that is not
        // JSON has no raw value.
        if self.include_raw_chunks {
            match &event {
                Ok(chunk) => controller.enqueue(StreamPart::Raw {
                    raw_value: chunk.clone(),
                }),
                Err(error) if error.is_recoverable_stream_error() => {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: Value::Null,
                    });
                }
                Err(_) => {}
            }
        }
        let Self {
            ref provider_options_name,
            ref url,
            ref request_body,
            ref response_headers,
            ref mut final_usage,
            ref mut has_tool_calls,
            ref mut finish_reason,
            ref mut active_reasoning_id,
            ref mut tool_calls,
            ..
        } = *self;
        match event {
            Ok(chunk) => {
                let chunk_type = chunk.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match chunk_type {
                    // -- Tool call / reasoning / message item added --
                    "response.output_item.added" => {
                        if let Some(item) = chunk.get("item") {
                            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match item_type {
                                "function_call" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    tool_calls.insert(
                                        id,
                                        ToolCallAccum {
                                            tool_name: item
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string),
                                            tool_call_id: item
                                                .get("call_id")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string),
                                            arguments: item
                                                .get("arguments")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string),
                                        },
                                    );
                                }
                                "reasoning" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    controller.reasoning_start(id.clone());
                                    *active_reasoning_id = Some(id);
                                }
                                "message" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    controller.text_start(id);
                                }
                                _ => {}
                            }
                        }
                    }
                    "response.function_call_arguments.delta" => {
                        let item_id = chunk
                            .get("item_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let delta = chunk
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        tool_calls
                            .entry(item_id)
                            .and_modify(|tc| {
                                let existing = tc.arguments.take().unwrap_or_default();
                                tc.arguments = Some(existing + &delta);
                            })
                            .or_insert(ToolCallAccum {
                                tool_name: None,
                                tool_call_id: None,
                                arguments: Some(delta),
                            });
                    }
                    "response.function_call_arguments.done" => {
                        let item_id = chunk
                            .get("item_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arguments = chunk
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        tool_calls
                            .entry(item_id)
                            .and_modify(|tc| {
                                tc.arguments = Some(arguments.clone());
                            })
                            .or_insert(ToolCallAccum {
                                tool_name: None,
                                tool_call_id: None,
                                arguments: Some(arguments),
                            });
                    }
                    "response.output_item.done" => {
                        if let Some(item) = chunk.get("item") {
                            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match item_type {
                                "function_call" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let accum = tool_calls.remove(&id);
                                    let tool_name = accum
                                        .as_ref()
                                        .and_then(|a| a.tool_name.clone())
                                        .or_else(|| {
                                            item.get("name")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string)
                                        })
                                        .unwrap_or_default();
                                    let tool_call_id = accum
                                        .as_ref()
                                        .and_then(|a| a.tool_call_id.clone())
                                        .or_else(|| {
                                            item.get("call_id")
                                                .and_then(|v| v.as_str())
                                                .map(std::string::ToString::to_string)
                                        })
                                        .unwrap_or_default();
                                    // The completed item's arguments win unless empty.
                                    let arguments = item
                                        .get("arguments")
                                        .and_then(Value::as_str)
                                        .filter(|arguments| !arguments.is_empty())
                                        .map(str::to_string)
                                        .or_else(|| {
                                            accum.as_ref().and_then(|a| a.arguments.clone())
                                        })
                                        .unwrap_or_default();
                                    let input = arguments;
                                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id,
                                        tool_name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        provider_metadata: item_id_metadata(
                                            provider_options_name,
                                            item,
                                        ),
                                    }));
                                    *has_tool_calls = true;
                                }
                                "reasoning" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if active_reasoning_id.as_ref() == Some(&id) {
                                        *active_reasoning_id = None;
                                    }
                                    controller.enqueue(StreamPart::ReasoningEnd {
                                        id,
                                        provider_metadata: reasoning_metadata(
                                            provider_options_name,
                                            item,
                                            &item["content"],
                                        ),
                                    });
                                }
                                "message" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let annotations = item["content"]
                                        .as_array()
                                        .map(|parts| output_text_annotations(parts))
                                        .unwrap_or_default();
                                    controller.enqueue(StreamPart::TextEnd {
                                        id,
                                        provider_metadata: message_metadata(
                                            provider_options_name,
                                            item,
                                            annotations,
                                        ),
                                    });
                                }
                                _ => {}
                            }
                        }
                    }

                    // -- Reasoning summary text delta, and reasoning text delta (LM Studio extension) --
                    "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                        let id = chunk
                            .get("item_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let delta = chunk
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        controller.reasoning_delta(id, delta);
                    }

                    // -- Text delta --
                    "response.output_text.delta" => {
                        let id = chunk
                            .get("item_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let delta = chunk
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        controller.text_delta(id, delta);
                    }

                    // -- Completion events --
                    "response.completed" | "response.incomplete" => {
                        if let Some(response) = chunk.get("response") {
                            let reason = response
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|r| r.as_str());
                            *finish_reason = FinishReason {
                                unified: map_open_responses_finish_reason(reason, *has_tool_calls),
                                raw: reason.map(std::string::ToString::to_string),
                            };
                            if let Some(usage_val) = response.get("usage") {
                                *final_usage = extract_usage_from_value(usage_val);
                            }
                        }
                    }
                    "response.failed" => {
                        if let Some(response) = chunk.get("response") {
                            let raw = response
                                .get("error")
                                .and_then(|e| e.get("code"))
                                .and_then(|c| c.as_str())
                                .or_else(|| response.get("status").and_then(|s| s.as_str()));
                            *finish_reason = FinishReason {
                                unified: FinishReasonUnified::Error,
                                raw: raw.map(std::string::ToString::to_string),
                            };
                            if let Some(usage_val) = response.get("usage") {
                                *final_usage = extract_usage_from_value(usage_val);
                            }
                            if let Some(error) = response.get("error").filter(|e| !e.is_null()) {
                                let error = stream_error(
                                    error,
                                    &chunk,
                                    url,
                                    request_body,
                                    response_headers,
                                );
                                controller.enqueue(StreamPart::Error { error });
                            }
                        }
                    }
                    "error" => {
                        let error = chunk.get("error").unwrap_or(&Value::Null);
                        *finish_reason = FinishReason {
                            unified: FinishReasonUnified::Error,
                            raw: error_code(error),
                        };
                        let error =
                            stream_error(error, &chunk, url, request_body, response_headers);
                        controller.enqueue(StreamPart::Error { error });
                    }
                    _ => {
                        // Ignore unrecognised event types.
                    }
                }
            }
            Err(error) => {
                if error.is_recoverable_stream_error() {
                    controller.enqueue_error(error);
                } else {
                    controller.error(error);
                }
            }
        }
    }

    fn flush(self, controller: &mut Controller) {
        if self.strict && !self.terminal_seen {
            controller.error(crate::stream::strict_error(
                "the stream ended without a response.completed, response.incomplete or \
                 response.failed event",
            ));
            return;
        }
        // Close any dangling reasoning segment.
        if let Some(id) = self.active_reasoning_id {
            controller.reasoning_end(id);
        }

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.finish_reason,
            usage: self.final_usage,
            provider_metadata: None,
        });
    }
}

// == Stream errors ==

/// createOpenResponsesStreamError: the error of a `response.failed` or
/// `error` event, with the event as `data` and no status code.
fn stream_error(
    error: &Value,
    event: &Value,
    url: &str,
    request_body: &Value,
    response_headers: &HashMap<String, String>,
) -> AiMuxError {
    aimux_provider_utils::create_provider_stream_error(
        aimux_provider_utils::ProviderStreamError {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error_code(error),
            status_code: None,
            is_retryable: None,
            data: event.clone(),
        },
        event,
        url,
        request_body.clone(),
        response_headers.clone(),
    )
}

/// An error's `code`, a string or a number.
fn error_code(error: &Value) -> Option<String> {
    match error.get("code")? {
        Value::String(code) => Some(code.clone()),
        Value::Number(code) => Some(code.to_string()),
        _ => None,
    }
}

// == Output provider metadata ==

/// `{ <name>: { itemId } }` of a tool call item.
fn item_id_metadata(provider_options_name: &str, item: &Value) -> Option<ProviderMetadata> {
    provider_namespace(provider_options_name, json!({ "itemId": item["id"] })).ok()
}

/// `{ <name>: { itemId, annotations? } }` of a message's text, with the
/// annotations only when there are any.
fn message_metadata(
    provider_options_name: &str,
    item: &Value,
    annotations: Vec<Value>,
) -> Option<ProviderMetadata> {
    let mut metadata = json!({ "itemId": item["id"] });
    if !annotations.is_empty() {
        metadata["annotations"] = json!(annotations);
    }
    provider_namespace(provider_options_name, metadata).ok()
}

/// The `url_citation` annotations of output text parts, flattened
/// (getOutputTextAnnotations): a part whose annotations are not all valid
/// citations contributes none.
fn output_text_annotations(parts: &[Value]) -> Vec<Value> {
    parts
        .iter()
        .filter_map(|part| part.get("annotations").and_then(url_citations))
        .flat_map(|annotations| match annotations {
            Value::Array(annotations) => annotations,
            _ => Vec::new(),
        })
        .collect()
}

/// createReasoningProviderMetadata: the item id, its summary, the given
/// reasoning content (`null` when absent) and the encrypted content.
fn reasoning_metadata(
    provider_options_name: &str,
    item: &Value,
    reasoning_content: &Value,
) -> Option<ProviderMetadata> {
    let texts = |parts: &Value, kind: &str| {
        json!(
            parts
                .as_array()
                .into_iter()
                .flatten()
                .map(|part| json!({ "type": kind, "text": part["text"] }))
                .collect::<Vec<_>>()
        )
    };
    let mut metadata = json!({
        "itemId": item["id"],
        "reasoningSummary": texts(&item["summary"], "summary_text"),
        "reasoningContent": if reasoning_content.is_array() {
            texts(reasoning_content, "reasoning_text")
        } else {
            Value::Null
        },
    });
    if let Some(encrypted) = item
        .get("encrypted_content")
        .filter(|value| !value.is_null())
    {
        metadata["reasoningEncryptedContent"] = encrypted.clone();
    }
    provider_namespace(provider_options_name, metadata).ok()
}

// == Tool call accumulator ==

struct ToolCallAccum {
    tool_name: Option<String>,
    tool_call_id: Option<String>,
    arguments: Option<String>,
}

// == Request body builder ==

/// Build the Open Responses request body and collect warnings.
///
/// Mirrors the TS `getArgs` method.
fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    provider_options_name: &str,
) -> (Value, Vec<Warning>) {
    let mut warnings = Vec::new();

    // Warnings for unsupported features.
    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".to_string(),
            details: None,
        });
    }
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".to_string(),
            details: None,
        });
    }
    if options.seed.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "seed".to_string(),
            details: None,
        });
    }

    // Convert prompt to input + instructions.
    let (input, instructions, input_warnings) =
        convert_to_open_responses_input_with_namespace(&options.prompt, provider_options_name);
    warnings.extend(input_warnings);

    // Convert function tools.
    let function_tools: Vec<Value> = options
        .tools
        .as_ref()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| match tool {
                    Tool::Function(ft) => {
                        let mut t = json!({
                            "type": "function",
                            "name": ft.name,
                            "parameters": ft.input_schema,
                        });
                        if let Some(desc) = &ft.description {
                            t["description"] = json!(desc);
                        }
                        if let Some(strict) = ft.strict {
                            t["strict"] = json!(strict);
                        }
                        Some(t)
                    }
                    // No extension encodes provider tools here (getArgs
                    // warns when none does).
                    Tool::Provider(tool) => {
                        warnings.push(Warning::Unsupported {
                            feature: format!("provider-defined tool {}", tool.id),
                            details: None,
                        });
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let converted_tool_choice: Option<Value> = match &options.tool_choice {
        None => None,
        Some(LanguageModelV4ToolChoice::Auto) => Some(json!("auto")),
        Some(LanguageModelV4ToolChoice::None) => Some(json!("none")),
        Some(LanguageModelV4ToolChoice::Required) => Some(json!("required")),
        // A provider tool's choice is omitted, as it has no encoding.
        Some(LanguageModelV4ToolChoice::Tool { tool_name }) => {
            let provider_tool = options
                .tools
                .iter()
                .flatten()
                .any(|tool| matches!(tool, Tool::Provider(tool) if &tool.name == tool_name));
            (!provider_tool).then(|| json!({ "type": "function", "name": tool_name }))
        }
    };

    // Convert response format (text format).
    let text_format: Option<Value> = match &options.response_format {
        Some(aimux_core::options::ResponseFormat::Json {
            schema,
            name,
            description,
        }) => {
            if schema.is_some() {
                let mut format = json!({
                    "type": "json_schema",
                    "strict": true,
                });
                if let Some(n) = name {
                    format["name"] = json!(n);
                } else {
                    format["name"] = json!("response");
                }
                if let Some(d) = description {
                    format["description"] = json!(d);
                }
                if let Some(s) = schema {
                    format["schema"] = s.clone();
                }
                Some(format)
            } else {
                Some(json!({ "type": "json_object" }))
            }
        }
        _ => None,
    };

    let open_responses_options = options
        .provider_options
        .as_ref()
        .and_then(|m| m.get(provider_options_name));

    // `providerOptions.<name>.reasoningEffort`, else the top-level reasoning.
    let provider_reasoning_effort = open_responses_options
        .and_then(|o| o.get("reasoningEffort"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let resolved_reasoning_effort: Option<String> = if provider_reasoning_effort.is_some() {
        provider_reasoning_effort
    } else if let Some(reasoning) = options.reasoning.filter(|r| r.is_custom()) {
        if reasoning == ReasoningEffort::None {
            Some("none".to_string())
        } else {
            aimux_provider_utils::map_reasoning_to_provider_effort(
                reasoning,
                &[
                    (ReasoningEffort::Minimal, "low"),
                    (ReasoningEffort::Low, "low"),
                    (ReasoningEffort::Medium, "medium"),
                    (ReasoningEffort::High, "high"),
                    (ReasoningEffort::Xhigh, "xhigh"),
                ],
                &mut warnings,
            )
            .map(str::to_owned)
        }
    } else {
        None
    };

    // Resolve reasoning summary from provider options.
    let reasoning_summary: Option<String> = open_responses_options
        .and_then(|o| o.get("reasoningSummary"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Build reasoning object.
    let reasoning: Option<Value> =
        if resolved_reasoning_effort.is_some() || reasoning_summary.is_some() {
            let mut r = Map::new();
            if let Some(effort) = resolved_reasoning_effort {
                r.insert("effort".to_string(), json!(effort));
            }
            if let Some(summary) = reasoning_summary {
                r.insert("summary".to_string(), json!(summary));
            }
            Some(Value::Object(r))
        } else {
            None
        };

    // Build the body - only insert non-None fields (matching TS undefined omission).
    let mut body = Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("input".to_string(), input);
    if let Some(instr) = instructions {
        body.insert("instructions".to_string(), json!(instr));
    }
    if let Some(max_tokens) = options.max_output_tokens {
        body.insert("max_output_tokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = options.temperature {
        body.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        body.insert("top_p".to_string(), json!(top_p));
    }
    if let Some(pp) = options.presence_penalty {
        body.insert("presence_penalty".to_string(), json!(pp));
    }
    if let Some(fp) = options.frequency_penalty {
        body.insert("frequency_penalty".to_string(), json!(fp));
    }
    if let Some(r) = reasoning {
        body.insert("reasoning".to_string(), r);
    }
    if !function_tools.is_empty() {
        body.insert("tools".to_string(), json!(function_tools));
    }
    if let Some(tc) = converted_tool_choice {
        body.insert("tool_choice".to_string(), tc);
    }
    if let Some(tf) = text_format {
        body.insert("text".to_string(), json!({ "format": tf }));
    }

    (Value::Object(body), warnings)
}

// == Usage extraction ==

/// Extract `Usage` from a response body `Value`.
fn extract_usage(raw: &Value) -> Usage {
    let usage_val = match raw.get("usage") {
        Some(u) if !u.is_null() => u,
        _ => return Usage::default(),
    };
    extract_usage_from_value(usage_val)
}

/// Extract `Usage` from a `usage` JSON value.
fn extract_usage_from_value(usage: &Value) -> Usage {
    let input_tokens = usage
        .get("input_tokens")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32);
    let cached_input_tokens = usage
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32);
    let reasoning_tokens = usage
        .get("output_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32);

    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: input_tokens,
            no_cache: Some(input_tokens.unwrap_or(0) - cached_input_tokens.unwrap_or(0)),
            cache_read: cached_input_tokens,
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: output_tokens,
            text: Some(output_tokens.unwrap_or(0) - reasoning_tokens.unwrap_or(0)),
            reasoning: reasoning_tokens,
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: usage.as_object().cloned(),
    }
}
