//! Cohere language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `cohere-chat-language-model.ts`. Cohere uses its own message
//! format (not OpenAI-compatible) and streams named SSE events
//! (`event: type\ndata: json`).

use aimux_core::tool::RawToolCall;
use async_trait::async_trait;
use serde_json::{Value, json};

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::SharedProviderOptions;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, ReasoningEffort, ResponseMetadata, Usage, Warning};

use aimux_provider_utils::{Transformer, pipe_through};

use crate::shared::EndpointConfig;
use crate::stream::{Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext};

use super::api_types::{ChatResponse, StreamEvent};
use super::convert_to_chat_prompt::convert_prompt_to_cohere;
use super::convert_usage::convert_usage;
use super::map_finish_reason::parse_finish_reason;
use super::prepare_tools::prepare_tools;
use std::sync::Arc;

/// A Cohere language model.
pub struct CohereModel {
    model_id: String,
    config: EndpointConfig,
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
}

impl CohereModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self {
            model_id,
            config,
            generate_id: Arc::new(aimux_provider_utils::generate_id),
        }
    }
    pub(crate) fn with_generate_id(
        mut self,
        generate_id: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    ) -> Self {
        if let Some(generate_id) = generate_id {
            self.generate_id = generate_id;
        }
        self
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

    /// Image URLs go to the API as they are
    /// (cohere-chat-language-model.ts:51-53).
    async fn supported_urls(&self) -> SupportedUrls {
        let http = regex::Regex::new(r"^https?://.*$").expect("static pattern");
        SupportedUrls(std::iter::once(("image/*".to_string(), vec![http])).collect())
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body_result = build_request_body(&self.model_id, options, false)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body_result.body.clone());
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(exchange.url("/chat"), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::error::cohere_failed_response_handler(),
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
                    super::api_types::ContentItem::Text { text } => {
                        if !text.is_empty() {
                            content.push(GenerateContent::Text {
                                text: text.clone(),
                                provider_metadata: None,
                            });
                        }
                    }
                    super::api_types::ContentItem::Thinking { thinking } => {
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
                    provider_metadata: Some(super::chat_language_model_options::cohere_metadata(
                        Value::Object(cohere_meta),
                    )),
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
        let body = exchange.transform_body(body_result.body.clone());
        let endpoint = exchange.url("/chat");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::error::cohere_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let sse_stream = resp.value;
        let stream_warnings = body_result.warnings;
        let stream = pipe_through(
            sse_stream,
            CohereChatStream {
                ctx: StreamContext {
                    warnings: stream_warnings,
                    include_raw_chunks: options.include_raw_chunks == Some(true),
                    strict: options.strict_stream == Some(true),
                    ..StreamContext::default()
                },
                ..CohereChatStream::default()
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

/// The `TransformStream` of `CohereChatLanguageModel.doStream`.
#[derive(Default)]
struct CohereChatStream {
    ctx: StreamContext,
    final_usage: Usage,
    /// `other` until a chunk says otherwise.
    finish_reason: Option<FinishReason>,
    pending_tool_call: Option<PendingToolCall>,
    is_reasoning: bool,
}

impl Transformer for CohereChatStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let raw = match event {
            Ok(raw) => raw,
            Err(error) => {
                if !error.is_recoverable_stream_error() {
                    controller.error(error);
                    return;
                }
                self.ctx.raw(&Value::Null, controller);
                self.finish_reason = Some(FINISH_ERROR);
                controller.enqueue(StreamPart::Error { error });
                return;
            }
        };
        self.ctx.raw(&raw, controller);
        let parsed = match StreamEvent::parse(raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.finish_reason = Some(FINISH_ERROR);
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
                    controller.reasoning_start(id);
                } else {
                    controller.text_start(id);
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
                    controller.text_delta(id, text);
                } else {
                    controller.reasoning_delta(id, content["thinking"].as_str().unwrap());
                }
            }
            "content-end" => {
                if self.is_reasoning {
                    controller.reasoning_end(id);
                    self.is_reasoning = false;
                } else {
                    controller.text_end(id);
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
                controller.tool_input_start(id.clone(), name);
                if !arguments.is_empty() {
                    controller.tool_input_delta(id, arguments);
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
                    controller.tool_input_delta(tool.id.clone(), delta);
                }
            }
            "tool-call-end" => {
                if let Some(tool) = self.pending_tool_call.take() {
                    controller.tool_input_end(tool.id.clone());
                    let text = tool.arguments.trim();
                    let input = match serde_json::from_str::<Value>(if text.is_empty() {
                        "{}"
                    } else {
                        text
                    }) {
                        Ok(value) if !contains_prototype_key(&value) => value.to_string(),
                        Ok(_) => {
                            controller.error(AiMuxError::invalid_response_data(
                                "Object contains forbidden prototype property",
                            ));
                            return;
                        }
                        Err(error) => {
                            controller.error(AiMuxError::JsonParse {
                                message: error.to_string(),
                                cause: Some(ErrorCause::source(&error)),
                            });
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
                self.finish_reason =
                    Some(parse_finish_reason(delta.finish_reason.as_ref().unwrap()));
                self.final_usage = convert_usage(delta.usage.as_ref().unwrap());
            }
            _ => {}
        }
    }

    fn flush(self, controller: &mut Controller) {
        if self.ctx.fail_without_terminal(
            self.finish_reason.is_some(),
            "a message-end event",
            controller,
        ) {
            return;
        }
        controller.enqueue(StreamPart::Finish {
            finish_reason: self.finish_reason.unwrap_or(FINISH_OTHER),
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

// ── Request body ────────────────────────────────────────────────────────────

/// Result of building a Cohere request body, including warnings collected from
/// tool preparation (provider-defined tools are unsupported).
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Convert `CallOptions` to a Cohere request body.
///
/// Mirrors the TS `getArgs` in `cohere-chat-language-model.ts`: assembles the
/// model id, messages, documents, sampling settings, response format, tools,
/// and the `thinking` config resolved from `reasoning` / provider options.
///
/// # Errors
/// Rejects unsupported prompt files and invalid provider options.
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    let converted = convert_prompt_to_cohere(&options.prompt)?;
    let mut warnings: Vec<Warning> = Vec::new();

    let mut body = json!({
        "model": model_id,
        "messages": converted.messages,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if !converted.documents.is_empty() {
        body["documents"] = json!(converted.documents);
    }

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = options.top_p {
        body["p"] = json!(top_p);
    }
    if let Some(top_k) = options.top_k {
        body["k"] = json!(top_k);
    }
    if let Some(seed) = options.seed {
        body["seed"] = json!(seed);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop_sequences"] = json!(stop);
    }
    if let Some(fp) = options.frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = options.presence_penalty {
        body["presence_penalty"] = json!(pp);
    }

    // Response format.
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json { schema, .. } => {
                let mut format = json!({ "type": "json_object" });
                if let Some(schema) = schema {
                    format["json_schema"] = schema.clone();
                }
                body["response_format"] = format;
            }
        }
    }

    // Tools.
    let prepared = prepare_tools(&options.tools, options.tool_choice.as_ref());
    warnings.extend(prepared.tool_warnings);
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }

    // Reasoning / thinking.
    if let Some(thinking) =
        resolve_cohere_thinking(options.reasoning, &options.provider_options, &mut warnings)?
    {
        body["thinking"] = thinking;
    }

    Ok(RequestBodyResult { body, warnings })
}

// ── Reasoning / thinking ────────────────────────────────────────────────────

/// Resolve the Cohere `thinking` config from the top-level `reasoning` option
/// and `providerOptions.cohere.thinking`.
///
/// Mirrors the TS `resolveCohereThinking`:
/// - `providerOptions.cohere.thinking` wins over top-level `reasoning`.
/// - `reasoning: None` (not specified) → no `thinking` field.
/// - `reasoning: "none"` → `{ type: "disabled" }`.
/// - other reasoning levels → `{ type: "enabled", token_budget: <n> }`.
///
/// # Errors
/// Rejects non-object thinking options, unknown types and nonnumeric budgets.
pub fn resolve_cohere_thinking(
    reasoning: Option<ReasoningEffort>,
    provider_options: &Option<SharedProviderOptions>,
    warnings: &mut Vec<Warning>,
) -> Result<Option<Value>, AiMuxError> {
    // Provider options take precedence.
    if let Some(cohere) =
        super::chat_language_model_options::cohere_options(provider_options.as_ref())
        && let Some(thinking) = cohere.get("thinking")
    {
        if !thinking.is_object() {
            return Err(AiMuxError::invalid_argument(
                "cohere.thinking must be an object",
            ));
        }
        let t_type = match thinking.get("type") {
            None => "enabled",
            Some(value) => match value.as_str() {
                Some(value @ ("enabled" | "disabled")) => value,
                _ => {
                    return Err(AiMuxError::invalid_argument(
                        "cohere.thinking.type must be enabled or disabled",
                    ));
                }
            },
        };
        let mut obj = json!({ "type": t_type });
        if let Some(budget) = thinking.get("tokenBudget") {
            if !budget.is_number() {
                return Err(AiMuxError::invalid_argument(
                    "cohere.thinking.tokenBudget must be a number",
                ));
            }
            obj["token_budget"] = budget.clone();
        }
        return Ok(Some(obj));
    }

    let Some(reasoning) = reasoning else {
        return Ok(None);
    };
    if reasoning == ReasoningEffort::ProviderDefault {
        return Ok(None);
    }
    if reasoning == ReasoningEffort::None {
        return Ok(Some(json!({ "type": "disabled" })));
    }

    let Some(budget) = aimux_provider_utils::map_reasoning_to_provider_budget(
        reasoning,
        32768,
        32768,
        aimux_provider_utils::DEFAULT_MIN_REASONING_BUDGET,
        aimux_provider_utils::DEFAULT_REASONING_BUDGET_PERCENTAGES,
        warnings,
    ) else {
        return Ok(None);
    };
    Ok(Some(json!({ "type": "enabled", "token_budget": budget })))
}
