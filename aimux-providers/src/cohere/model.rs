//! Cohere language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `cohere-chat-language-model.ts`. Cohere uses its own message
//! format (not OpenAI-compatible) and streams named SSE events
//! (`event: type\ndata: json`).

use aimux_core::tool::RawToolCall;
use async_trait::async_trait;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage, Warning};

use aimux_provider_utils::{TransformStreamController, Transformer, pipe_through};

use crate::shared::EndpointConfig;

use super::convert::{build_request_body, parse_finish_reason};
use super::types::{ChatResponse, StreamEvent, UsageResponse};

/// A Cohere language model.
pub struct CohereModel {
    model_id: String,
    config: EndpointConfig,
    generate_id: aimux_provider_utils::IdGenerator,
}

impl CohereModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self {
            model_id,
            config,
            generate_id: aimux_provider_utils::generate_id,
        }
    }
    pub(crate) fn with_generate_id(
        mut self,
        generate_id: Option<aimux_provider_utils::IdGenerator>,
    ) -> Self {
        if let Some(generate_id) = generate_id {
            self.generate_id = generate_id;
        }
        self
    }
}

// ── Usage conversion ─────────────────────────────────────────────────────────

/// Convert Cohere `tokens` usage into the core `Usage` type.
///
/// Mirrors the TS `convertCohereUsage`:
/// - `input.total = input_tokens`, `input.noCache = input_tokens`
/// - `output.total = output_tokens`
fn convert_usage(usage: &UsageResponse) -> Usage {
    let tokens = &usage.tokens;
    Usage {
        input_tokens: aimux_core::types::InputTokenUsage {
            total: Some(tokens.input_tokens),
            no_cache: Some(tokens.input_tokens),
            cache_read: None,
            cache_write: None,
        },
        output_tokens: aimux_core::types::OutputTokenUsage {
            total: Some(tokens.output_tokens),
            text: Some(tokens.output_tokens),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: usage.raw.as_object().cloned(),
    }
}

// ── Pending tool call accumulator (streaming) ────────────────────────────────

struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

// ── LanguageModel impl ───────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for CohereModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body_result = build_request_body(&self.model_id, options, false)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = body_result.body.clone();
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/chat"), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::cohere_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let data: ChatResponse = resp.value;

        // Build content array.
        let mut content = Vec::new();

        // Content items (text and thinking).
        if let Some(items) = &data.message.content {
            for item in items {
                match item {
                    super::types::ContentItem::Text { text } => {
                        if !text.is_empty() {
                            content.push(GenerateContent::Text {
                                text: text.clone(),
                                provider_metadata: None,
                            });
                        }
                    }
                    super::types::ContentItem::Thinking { thinking } => {
                        // Mirrors TS: a `thinking` content item becomes a
                        // `reasoning` content item. Empty thinking is dropped.
                        if !thinking.is_empty() {
                            content.push(GenerateContent::Reasoning(ReasoningOutput {
                                text: thinking.clone(),
                                provider_metadata: None,
                            }));
                        }
                    }
                }
            }
        }

        // Citations (RAG) → Source content items.
        //
        // Mirrors TS `cohere-chat-language-model.ts`: each citation becomes a
        // `{ type: 'source', sourceType: 'document', title, providerMetadata:
        // { cohere: { start, end, text, sources, citationType } } }` content
        // item. The per-citation metadata (start/end/text/sources/citationType)
        // is preserved in `provider_metadata`.
        if let Some(citations) = &data.message.citations {
            for citation in citations {
                let title = citation
                    .get("sources")
                    .and_then(|s| s.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|src| src.get("document"))
                    .and_then(|d| d.get("title"))
                    .and_then(|t| t.as_str())
                    .filter(|title| !title.is_empty())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| "Document".to_string());

                // Build `providerMetadata.cohere` from the citation's fields.
                // Only present fields are included (matching TS, where `undefined`
                // values are dropped); `citationType` mirrors the TS spread
                // `...(citation.type && { citationType: citation.type })` and is
                // included only when `citation.type` is a non-empty string.
                let mut cohere_meta = serde_json::Map::new();
                for field in ["start", "end", "text", "sources"] {
                    if let Some(v) = citation.get(field) {
                        cohere_meta.insert(field.to_string(), v.clone());
                    }
                }
                if let Some(t) = citation
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    cohere_meta.insert("citationType".to_string(), Value::String(t.to_string()));
                }
                content.push(GenerateContent::Source(Source::Document {
                    id: (self.generate_id)(),
                    media_type: "text/plain".to_string(),
                    title,
                    filename: None,
                    provider_metadata: Some(super::options::cohere_metadata(Value::Object(
                        cohere_meta,
                    ))),
                }));
            }
        }

        // Tool calls.
        if let Some(tool_calls) = &data.message.tool_calls {
            for tc in tool_calls {
                // Cohere returns the literal string "null" for tools defined
                // as having no arguments (TS: `.replace(/^null$/, '{}')`).
                let args_str = if tc.function.arguments == "null" {
                    "{}".to_string()
                } else {
                    tc.function.arguments.clone()
                };
                let input = args_str;
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.function.name.clone(),
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
        }

        let finish_reason = parse_finish_reason(&data.finish_reason);
        let usage = convert_usage(&data.usage);

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: body_result.warnings,
            provider_metadata: None,
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.generation_id,
                timestamp: None,
                model_id: None,
                headers: Some(response_headers),
                body: resp.raw_value,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body_result = build_request_body(&self.model_id, options, true)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = body_result.body.clone();
        let endpoint = exchange.url("/chat");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::cohere_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let sse_stream = resp.value;
        let stream_warnings = body_result.warnings;
        let include_raw_chunks = options.include_raw_chunks.unwrap_or(false);
        let stream = pipe_through(
            sse_stream,
            CohereChatStream {
                warnings: stream_warnings,
                include_raw_chunks,
                final_usage: Usage::default(),
                finish_reason: FinishReason {
                    unified: FinishReasonUnified::Other,
                    raw: None,
                },
                pending_tool_call: None,
                is_reasoning: false,
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

/// The `TransformStream` of `CohereChatLanguageModel.doStream`.
struct CohereChatStream {
    warnings: Vec<Warning>,
    include_raw_chunks: bool,
    final_usage: Usage,
    finish_reason: FinishReason,
    pending_tool_call: Option<PendingToolCall>,
    is_reasoning: bool,
}

impl Transformer for CohereChatStream {
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
        let raw = match event {
            Ok(raw) => raw,
            Err(error) => {
                if !error.is_recoverable_stream_error() {
                    controller.error(error);
                    return;
                }
                if self.include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: Value::Null,
                    });
                }
                self.finish_reason = FinishReason {
                    unified: FinishReasonUnified::Error,
                    raw: None,
                };
                controller.enqueue(StreamPart::Error { error });
                return;
            }
        };
        if self.include_raw_chunks {
            controller.enqueue(StreamPart::Raw {
                raw_value: raw.clone(),
            });
        }
        let parsed = match StreamEvent::parse(raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.finish_reason = FinishReason {
                    unified: FinishReasonUnified::Error,
                    raw: None,
                };
                controller.enqueue(StreamPart::Error { error });
                return;
            }
        };
        let id = parsed
            .index
            .map(|index| index.to_string())
            .unwrap_or_default();
        match parsed.event_type.as_str() {
            "message-start" => controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                id: parsed.id,
                timestamp: None,
                model_id: None,
            })),
            "content-start" => {
                let content = parsed
                    .delta
                    .as_ref()
                    .and_then(|delta| delta.message.as_ref())
                    .and_then(|message| message.content.as_ref())
                    .unwrap();
                if content["type"] == "thinking" {
                    self.is_reasoning = true;
                    controller.enqueue(StreamPart::ReasoningStart {
                        id,
                        provider_metadata: None,
                    });
                } else {
                    controller.enqueue(StreamPart::TextStart {
                        id,
                        provider_metadata: None,
                    });
                }
            }
            "content-delta" => {
                let content = parsed
                    .delta
                    .as_ref()
                    .and_then(|delta| delta.message.as_ref())
                    .and_then(|message| message.content.as_ref())
                    .unwrap();
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    controller.enqueue(StreamPart::TextDelta {
                        id,
                        delta: text.into(),
                        provider_metadata: None,
                    });
                } else {
                    controller.enqueue(StreamPart::ReasoningDelta {
                        id,
                        delta: content["thinking"].as_str().unwrap().into(),
                        provider_metadata: None,
                    });
                }
            }
            "content-end" => {
                if self.is_reasoning {
                    controller.enqueue(StreamPart::ReasoningEnd {
                        id,
                        provider_metadata: None,
                    });
                    self.is_reasoning = false;
                } else {
                    controller.enqueue(StreamPart::TextEnd {
                        id,
                        provider_metadata: None,
                    });
                }
            }
            "tool-call-start" => {
                let tool = parsed
                    .delta
                    .as_ref()
                    .and_then(|delta| delta.message.as_ref())
                    .and_then(|message| message.tool_calls.as_ref())
                    .unwrap();
                let id = tool["id"].as_str().unwrap().to_string();
                let name = tool["function"]["name"].as_str().unwrap().to_string();
                let arguments = tool["function"]["arguments"].as_str().unwrap().to_string();
                self.pending_tool_call = Some(PendingToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                });
                controller.enqueue(StreamPart::ToolInputStart {
                    id: id.clone(),
                    tool_name: name,
                    provider_executed: None,
                    dynamic: None,
                    title: None,
                    provider_metadata: None,
                });
                if !arguments.is_empty() {
                    controller.enqueue(StreamPart::ToolInputDelta {
                        id,
                        delta: arguments,
                        provider_metadata: None,
                    });
                }
            }
            "tool-call-delta" => {
                if let Some(tool) = &mut self.pending_tool_call {
                    let delta = parsed
                        .delta
                        .as_ref()
                        .and_then(|delta| delta.message.as_ref())
                        .and_then(|message| message.tool_calls.as_ref())
                        .unwrap()["function"]["arguments"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    tool.arguments.push_str(&delta);
                    controller.enqueue(StreamPart::ToolInputDelta {
                        id: tool.id.clone(),
                        delta,
                        provider_metadata: None,
                    });
                }
            }
            "tool-call-end" => {
                if let Some(tool) = self.pending_tool_call.take() {
                    controller.enqueue(StreamPart::ToolInputEnd {
                        id: tool.id.clone(),
                        provider_metadata: None,
                    });
                    let text = tool.arguments.trim();
                    let input = match serde_json::from_str::<Value>(if text.is_empty() {
                        "{}"
                    } else {
                        text
                    }) {
                        Ok(value) if !contains_prototype_key(&value) => value.to_string(),
                        Ok(_) => {
                            controller.error(AiMuxError::InvalidResponseData(
                                "Object contains forbidden prototype property".into(),
                            ));
                            return;
                        }
                        Err(error) => {
                            controller.error(AiMuxError::JsonParse(error.to_string()));
                            return;
                        }
                    };
                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                        tool_call_id: tool.id,
                        tool_name: tool.name,
                        input,
                        provider_executed: None,
                        dynamic: None,
                        provider_metadata: None,
                    }));
                }
            }
            "message-end" => {
                let delta = parsed.delta.unwrap();
                self.finish_reason = parse_finish_reason(delta.finish_reason.as_ref().unwrap());
                self.final_usage = convert_usage(delta.usage.as_ref().unwrap());
            }
            _ => {}
        }
    }

    fn flush(self, controller: &mut TransformStreamController<StreamPart>) {
        controller.enqueue(StreamPart::Finish {
            finish_reason: self.finish_reason,
            usage: self.final_usage,
            provider_metadata: None,
        });
    }
}

fn contains_prototype_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            key == "__proto__"
                || (key == "constructor" && value.is_object() && value.get("prototype").is_some())
                || contains_prototype_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_prototype_key),
        _ => false,
    }
}
