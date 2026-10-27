//! Mistral language model — implements `LanguageModel` trait.
//!
//! Mirrors the TS `mistral-chat-language-model.ts`. Key differences from the
//! OpenAI model:
//! - `content` in responses can be a string or an array of typed parts
//!   (text, thinking, image_url). Thinking parts are extracted as reasoning
//!   in streaming mode.
//! - Streamed tool calls are assembled by the shared `ChatToolCalls`.
//! - Usage supports `num_cached_tokens` / `prompt_tokens_details.cached_tokens`.
//! - Finish reasons include `model_length`.

use aimux_core::tool::{RawToolCall, Tool};

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, ReasoningEffort, ResponseMetadata, Usage, Warning};

use aimux_provider_utils::{Transformer, inject_json_instruction_into_messages, pipe_through};

use crate::shared::EndpointConfig;
use crate::stream::{ChatToolCalls, Controller, Emit, FINISH_OTHER, StreamContext, ToolCallDelta};

use super::api_types::{ChatCompletionResponse, StreamChunk};
use super::convert_to_chat_messages::convert_prompt_to_mistral_messages;
use super::convert_usage::convert_usage;
use super::map_finish_reason::parse_finish_reason;
use super::prepare_tools::prepare_tools;

/// A Mistral language model.
pub struct MistralModel {
    model_id: String,
    config: EndpointConfig,
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
}

impl MistralModel {
    pub(crate) fn from_config(
        model_id: String,
        config: EndpointConfig,
        generate_id_fn: Option<Arc<dyn Fn() -> String + Send + Sync>>,
    ) -> Self {
        Self {
            model_id,
            config,
            generate_id: generate_id_fn
                .unwrap_or_else(|| Arc::new(aimux_provider_utils::generate_id)),
        }
    }
}

// ── Content extraction helpers ───────────────────────────────────────────────

/// Extract text content from a Mistral `content` field (string or array).
///
/// When the content is an array, only `text` parts are joined; `thinking`,
/// `image_url`, and `reference` parts are skipped.
fn extract_text_content(content: &Option<Value>) -> Option<String> {
    match content {
        None => None,
        Some(Value::String(s)) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        Some(Value::Array(arr)) => {
            let text: String = arr
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                        part.get("text")
                            .and_then(|t| t.as_str())
                            .map(std::string::ToString::to_string)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() { None } else { Some(text) }
        }
        _ => None,
    }
}

/// Extract the text chunks of one thinking part.
fn extract_reasoning_part(part: &Value) -> Option<String> {
    if part.get("type").and_then(Value::as_str) != Some("thinking") {
        return None;
    }
    let text: String = part
        .get("thinking")?
        .as_array()?
        .iter()
        .filter(|chunk| chunk.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|chunk| chunk.get("text").and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then_some(text)
}

// ── LanguageModel impl ───────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for MistralModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    async fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
        self.config.supported_urls(&self.model_id)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(build_request_body(&self.model_id, options, false)?);
        let endpoint = exchange.url("/chat/completions");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler(),
            super::error::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let data: ChatCompletionResponse = resp.value;

        let choice = data
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AiMuxError::invalid_response_data("no choices in response"))?;

        // Build content array.
        let mut content = Vec::new();

        if let Some(Value::Array(parts)) = &choice.message.content {
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("thinking") => {
                        if let Some(text) = extract_reasoning_part(part) {
                            content.push(GenerateContent::Reasoning(ReasoningOutput {
                                text,
                                provider_metadata: None,
                            }));
                        }
                    }
                    Some("text") => {
                        if let Some(text) = part
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                        {
                            content.push(GenerateContent::Text {
                                text: text.to_string(),
                                provider_metadata: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        } else if let Some(text) = extract_text_content(&choice.message.content) {
            content.push(GenerateContent::Text {
                text,
                provider_metadata: None,
            });
        }

        // Tool calls.
        if let Some(tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input = tc.function.arguments;
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: tc.id,
                    tool_name: tc.function.name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
        }

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FINISH_OTHER);

        let usage = convert_usage(&data.usage);

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request_warnings(options, &self.model_id),
            provider_metadata: None,
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.id,
                timestamp: data
                    .created
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
                model_id: data.model,
                headers: Some(response_headers),
                body: resp.raw_value,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(build_request_body(&self.model_id, options, true)?);
        let endpoint = exchange.url("/chat/completions");
        let resp = aimux_provider_utils::post_json_to_api(
            exchange.request(endpoint.clone(), options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::error::mistral_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let stream = pipe_through(
            resp.value,
            MistralChatStream::new(StreamContext {
                generate_id: self.generate_id.clone(),
                ..StreamContext::new(
                    options,
                    request_warnings(options, &self.model_id),
                    endpoint,
                    &body,
                    &response_headers,
                )
            }),
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

/// The id of the (single) text block of a streamed response.
const TEXT_ID: &str = "0";

/// The `TransformStream` of the Mistral chat model's `doStream`.
#[derive(Default)]
struct MistralChatStream {
    /// Its id generator names reasoning blocks and id-less tool calls.
    ctx: StreamContext,
    tool_calls: ChatToolCalls,
    text_started: bool,
    /// The open reasoning block.
    reasoning_id: Option<String>,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    response_metadata_emitted: bool,
}

/// A chat stream with an empty request context and tool-call ids from
/// `generate_id`, for tests that drive it chunk by chunk.
#[cfg(test)]
pub(crate) fn chat_test_stream(
    generate_id: Arc<dyn Fn() -> String + Send + Sync>,
) -> impl Transformer<Input = Result<Value, AiMuxError>, Output = StreamPart> {
    MistralChatStream::new(StreamContext {
        generate_id,
        ..StreamContext::default()
    })
}

impl MistralChatStream {
    fn new(ctx: StreamContext) -> Self {
        Self {
            tool_calls: ChatToolCalls::new(ctx.generate_id.clone()),
            ctx,
            ..Self::default()
        }
    }

    /// Handle one parsed SSE `data:` payload.
    fn transform_chunk(&mut self, parsed: Value, controller: &mut Controller) {
        self.ctx.raw(&parsed, controller);

        if let Some(err_obj) = parsed.get("error") {
            controller.enqueue(StreamPart::Error {
                error: super::error::mistral_stream_error(
                    err_obj,
                    &self.ctx.url,
                    self.ctx.request_body.clone(),
                    self.ctx.response_headers.clone(),
                ),
            });
            return;
        }

        let chunk: StreamChunk = match serde_json::from_value(parsed) {
            Ok(c) => c,
            Err(e) => {
                controller.enqueue(StreamPart::Error { error: e.into() });
                return;
            }
        };

        // Emit ResponseMetadata from the first valid chunk.
        if !self.response_metadata_emitted {
            self.response_metadata_emitted = true;
            controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
                id: chunk.id.clone(),
                timestamp: chunk
                    .created
                    .and_then(|secs| aimux_core::Timestamp::from_unix_seconds(secs as f64)),
                model_id: chunk.model.clone(),
            }));
        }

        // Update usage.
        if let Some(usage) = &chunk.usage {
            self.final_usage = convert_usage(usage);
        }

        // Upstream processes only the first choice.
        let Some(choice) = chunk.choices.into_iter().next() else {
            return;
        };

        // Reasoning content (from thinking parts in array content).
        for reasoning_delta in choice
            .delta
            .content
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(extract_reasoning_part)
        {
            if self.reasoning_id.is_none() {
                // End any active text before starting reasoning.
                if self.text_started {
                    controller.text_end(TEXT_ID);
                    self.text_started = false;
                }
                let rid = (self.ctx.generate_id)();
                self.reasoning_id = Some(rid.clone());
                controller.reasoning_start(rid);
            }
            if let Some(ref rid) = self.reasoning_id {
                controller.reasoning_delta(rid.clone(), reasoning_delta);
            }
        }

        // Text content.
        if let Some(text_delta) = extract_text_content(&choice.delta.content)
            && !text_delta.is_empty()
        {
            if !self.text_started {
                // End reasoning before starting text.
                if let Some(rid) = self.reasoning_id.take() {
                    controller.reasoning_end(rid);
                }
                controller.text_start(TEXT_ID);
                self.text_started = true;
            }
            controller.text_delta(TEXT_ID, text_delta);
        }

        // Tool calls: assembled by the shared tracker.
        if let Some(tool_call_deltas) = choice.delta.tool_calls {
            for dtc in &tool_call_deltas {
                let delta = ToolCallDelta {
                    index: dtc.index,
                    id: dtc.id.as_deref(),
                    name: dtc.function.name.as_deref(),
                    arguments: dtc.function.arguments.as_deref(),
                    ..ToolCallDelta::default()
                };
                // A new call without a function name is invalid response
                // data, as in the AI SDK; the stream ends.
                if !self.tool_calls.delta(delta, controller) {
                    return;
                }
            }
        }

        // Finish reason.
        if let Some(reason) = choice.finish_reason {
            self.final_finish_reason = Some(parse_finish_reason(&reason));
        }
    }
}

impl Transformer for MistralChatStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        match event {
            Ok(parsed) => self.transform_chunk(parsed, controller),
            Err(error) => {
                self.ctx.raw(&Value::Null, controller);
                controller.enqueue(StreamPart::Error { error });
            }
        }
    }

    fn flush(mut self, controller: &mut Controller) {
        // Close any remaining open segments.
        if self.text_started {
            controller.text_end(TEXT_ID);
        }
        if let Some(rid) = self.reasoning_id.take() {
            controller.reasoning_end(rid);
        }

        self.tool_calls.finish(controller);

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FINISH_OTHER),
            usage: self.final_usage,
            provider_metadata: None,
        });
    }
}

// ── Request body ────────────────────────────────────────────────────────────

/// Convert `CallOptions` to a Mistral request body.
///
/// # Errors
/// Rejects invalid provider options and unsupported prompt parts.
pub fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<Value, AiMuxError> {
    let provider_options = super::chat_language_model_options::validated_mistral_options(
        options.provider_options.as_ref(),
    )?;
    let structured_outputs = provider_options
        .and_then(|o| o.get("structuredOutputs"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    // Mistral's JSON object mode needs the instruction in the prompt
    // (@ai-sdk/mistral@4.0.55 mistral-chat-language-model.ts:171-181).
    let messages = match &options.response_format {
        Some(ResponseFormat::Json { schema, .. }) if !structured_outputs || schema.is_none() => {
            convert_prompt_to_mistral_messages(&inject_json_instruction_into_messages(
                &options.prompt,
                schema.as_ref(),
                None,
                None,
            ))?
        }
        _ => convert_prompt_to_mistral_messages(&options.prompt)?,
    };

    let mut body = json!({
        "model": model_id,
        "messages": messages,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = options.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(ref stop) = options.stop_sequences {
        body["stop"] = json!(stop);
    }
    if let Some(seed) = options.seed {
        body["random_seed"] = json!(seed);
    }
    if let Some(fp) = options.frequency_penalty {
        body["frequency_penalty"] = json!(fp);
    }
    if let Some(pp) = options.presence_penalty {
        body["presence_penalty"] = json!(pp);
    }

    // Response format — Mistral uses json_schema / json_object.
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description,
            } => {
                if schema.is_some() && structured_outputs {
                    let mut schema_obj = json!({});
                    if let Some(s) = schema {
                        schema_obj["schema"] = s.clone();
                    }
                    schema_obj["name"] =
                        json!(name.clone().unwrap_or_else(|| "response".to_string()));
                    if let Some(d) = description {
                        schema_obj["description"] = json!(d);
                    }
                    schema_obj["strict"] = json!(
                        provider_options
                            .and_then(|o| o.get("strictJsonSchema"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    );
                    body["response_format"] = json!({
                        "type": "json_schema",
                        "json_schema": schema_obj,
                    });
                } else {
                    body["response_format"] = json!({ "type": "json_object" });
                }
            }
        }
    }

    if let Some(provider_options) = provider_options {
        for (key, wire_key) in [
            ("safePrompt", "safe_prompt"),
            ("documentImageLimit", "document_image_limit"),
            ("documentPageLimit", "document_page_limit"),
            ("promptCacheKey", "prompt_cache_key"),
        ] {
            if let Some(value) = provider_options.get(key) {
                body[wire_key] = value.clone();
            }
        }
    }
    if supports_reasoning_effort(model_id) {
        let effort = provider_options
            .and_then(|o| o.get("reasoningEffort"))
            .cloned()
            .or_else(|| {
                options.reasoning.filter(|r| r.is_custom()).map(|r| {
                    json!(if r == ReasoningEffort::None {
                        "none"
                    } else {
                        "high"
                    })
                })
            });
        if let Some(effort) = effort {
            body["reasoning_effort"] = effort;
        }
    }

    // Preserve the empty tools array when the supplied tools are all unsupported.
    if let Some(tools) = options.tools.as_ref().filter(|tools| !tools.is_empty()) {
        let function_tools = Some(
            tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => Some(ft.clone()),
                    Tool::Provider(_) => None,
                })
                .collect(),
        );
        let mut prepared = prepare_tools(&function_tools, options.tool_choice.as_ref());
        if prepared.tools.is_none() {
            prepared.tools = Some(Vec::new());
            prepared.tool_choice = options.tool_choice.as_ref().map(|choice| {
                json!(match choice {
                    LanguageModelV4ToolChoice::Auto => "auto",
                    LanguageModelV4ToolChoice::None => "none",
                    _ => "any",
                })
            });
        }
        body["tools"] = json!(prepared.tools);
        if let Some(choice) = prepared.tool_choice {
            body["tool_choice"] = choice;
        }
        if let Some(value) = provider_options.and_then(|o| o.get("parallelToolCalls")) {
            body["parallel_tool_calls"] = value.clone();
        }
    }
    Ok(body)
}

fn supports_reasoning_effort(model_id: &str) -> bool {
    matches!(
        model_id,
        "glm-5-2"
            | "labs-leanstral-1-5"
            | "labs-leanstral-1-5-1"
            | "magistral-medium-latest"
            | "magistral-small-latest"
            | "mistral-medium"
            | "mistral-medium-2604"
            | "mistral-medium-3"
            | "mistral-medium-3-5"
            | "mistral-medium-3.5"
            | "mistral-medium-latest"
            | "mistral-small-2603"
            | "mistral-small-latest"
            | "mistral-vibe-cli-fast"
            | "mistral-vibe-cli-latest"
            | "mistral-vibe-cli-with-tools"
            | "zai-glm-5-2"
    )
}

/// The warnings emitted while preparing an upstream Mistral request.
#[must_use]
pub fn request_warnings(options: &CallOptions, model_id: &str) -> Vec<Warning> {
    let mut warnings = Vec::new();
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".into(),
            details: None,
        });
    }
    if let Some(reasoning) = options.reasoning.filter(|r| r.is_custom()) {
        if !supports_reasoning_effort(model_id) {
            warnings.push(Warning::Unsupported {
                feature: "reasoning".into(),
                details: Some("This model does not support reasoning configuration.".into()),
            });
        } else if !matches!(reasoning, ReasoningEffort::None | ReasoningEffort::High)
            && super::chat_language_model_options::mistral_options(
                options.provider_options.as_ref(),
            )
            .and_then(|o| o.get("reasoningEffort"))
            .is_none()
        {
            warnings.push(Warning::Compatibility { feature:"reasoning".into(), details:Some(format!("reasoning \"{reasoning}\" is not directly supported by this model. mapped to effort \"high\".")) });
        }
    }
    for tool in options.tools.iter().flatten() {
        if let Tool::Provider(tool) = tool {
            warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", tool.id),
                details: None,
            });
        }
    }
    warnings
}
