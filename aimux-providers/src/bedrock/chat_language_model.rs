//! Amazon Bedrock language model — implements `LanguageModel`.
//!
//! Uses the Bedrock Converse API (`/model/{model-id}/converse` and
//! `/model/{model-id}/converse-stream`), which provides a unified interface
//! across all Bedrock-backed models (Anthropic Claude, Meta Llama, Mistral,
//! etc.).

use super::anthropic_model_support::{supports_native_structured_output, supports_strict_tools};
use super::api_types::{BedrockContentBlock, BedrockConverseResponse};
use super::chat_language_model_options::validate_options;
use super::convert_to_chat_messages::convert_prompt;
use super::convert_usage::convert_usage;
use super::event_stream_decoder::EventStreamMessage;
use super::event_stream_response_handler::bedrock_event_stream_response_handler;
use super::map_finish_reason::map_finish_reason;
use super::normalize_tool_call_id::normalize_tool_call_id;
use super::options;
use super::prepare_tools::{is_strict_schema_compatible, prepare_tools};
use crate::shared::EndpointConfig;
use crate::stream::{Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext};
use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage};
use aimux_core::options::{CallOptions, LanguageModelV4ToolChoice, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{FunctionTool, RawToolCall, Tool};
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage,
};
use aimux_provider_utils::{
    HttpBody, Transformer, inject_json_instruction_into_messages, pipe_through,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// An Amazon Bedrock language model (e.g. `anthropic.claude-3-5-sonnet-20240620-v1:0`).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct BedrockModel {
    model_id: String,
    config: EndpointConfig,
    pub(super) model_family: Option<String>,
}

impl BedrockModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self {
            model_id,
            config,
            model_family: None,
        }
    }

    pub(crate) fn with_model_family(mut self, model_family: Option<String>) -> Self {
        self.model_family = model_family;
        self
    }

    /// `/model/{model-id}/converse` or `/converse-stream`.
    fn path(&self, stream: bool) -> String {
        let suffix = if stream {
            "converse-stream"
        } else {
            "converse"
        };
        format!(
            "/model/{}/{}",
            super::provider::encode_model_id(&self.model_id),
            suffix
        )
    }
}

#[async_trait]
impl LanguageModel for BedrockModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn supported_urls(&self) -> SupportedUrls {
        let s3 = regex::Regex::new(r"^s3://").expect("static pattern");
        SupportedUrls(
            [
                ("image/*".into(), vec![s3.clone()]),
                ("video/*".into(), vec![s3]),
            ]
            .into_iter()
            .collect(),
        )
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let (body, warnings, uses_json_instruction, uses_json_tool) =
            build_request_body_checked(&self.model_id, options, self.model_family.as_deref())?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = exchange.url(&self.path(false));
        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_json_response_handler(),
            super::error::bedrock_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let data: BedrockConverseResponse = resp.value;

        // Extract content from response.output.message.content
        let mut content = Vec::new();
        let mut extractor = uses_json_instruction.then(JsonObjectTextExtractor::default);
        let mut is_json_response_from_tool = false;
        if let Some(output) = &data.output
            && let Some(message) = &output.message
        {
            for block in &message.content {
                let before = content.len();
                extract_content(block, &mut content, &self.model_id);
                for part in &mut content[before..] {
                    if let GenerateContent::ToolCall(tool) = part
                        && uses_json_tool
                        && tool.tool_name == "json"
                    {
                        is_json_response_from_tool = true;
                        *part = GenerateContent::Text {
                            text: tool.input.clone(),
                            provider_metadata: None,
                        };
                    }
                }
                if let Some(extractor) = &mut extractor {
                    for part in &mut content[before..] {
                        if let GenerateContent::Text { text, .. } = part {
                            *text = extractor.process(text);
                        }
                    }
                }
            }
        }

        let mut finish_reason = data
            .stop_reason
            .as_deref()
            .map(map_finish_reason)
            .unwrap_or(FINISH_OTHER);

        if is_json_response_from_tool && finish_reason.unified == FinishReasonUnified::ToolCalls {
            finish_reason.unified = FinishReasonUnified::Stop;
        }
        let mut provider_metadata = response_provider_metadata(&data);
        if is_json_response_from_tool {
            let mut payload = provider_metadata
                .as_ref()
                .and_then(|meta| meta.get(options::AMAZON_BEDROCK))
                .cloned()
                .unwrap_or_default();
            payload.insert("isJsonResponseFromTool".into(), json!(true));
            payload
                .entry("stopSequence")
                .or_insert(serde_json::Value::Null);
            provider_metadata = Some(options::metadata(json!(payload)));
        }
        let usage = convert_usage(data.usage.as_ref());

        let request_id = response_headers
            .get("x-amzn-requestid")
            .cloned()
            .or_else(|| response_headers.get("x-amzn-request-id").cloned());

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata,
            response: Some(aimux_core::shared::ResponseInfo {
                id: request_id,
                timestamp: response_headers
                    .get("date")
                    .and_then(|date| aimux_core::Timestamp::from_http_date(date)),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: None,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let (body, warnings, uses_json_instruction, uses_json_tool) =
            build_request_body_checked(&self.model_id, options, self.model_family.as_deref())?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = exchange.url(&self.path(true));
        let error_url = url.clone();
        let error_body = body.clone();
        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            bedrock_event_stream_response_handler(),
            super::error::bedrock_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let body_stream = resp.value;
        let request_id = response_headers
            .get("x-amzn-requestid")
            .cloned()
            .or_else(|| response_headers.get("x-amzn-request-id").cloned());
        // Same source as the non-stream path: the response `Date` header
        // (RFC1123). Keeps stream/non-stream timestamps consistent.
        let response_timestamp = response_headers
            .get("date")
            .and_then(|date| aimux_core::Timestamp::from_http_date(date));

        let stream = pipe_through(
            super::event_stream_decoder::decode_stream(body_stream),
            AmazonBedrockChatStream {
                ctx: StreamContext::new(
                    options,
                    warnings,
                    error_url,
                    &error_body,
                    &response_headers,
                ),
                request_id,
                response_timestamp,
                model_id: self.model_id.clone(),
                uses_json_tool,
                extractor: uses_json_instruction.then(JsonObjectTextExtractor::default),
                ..AmazonBedrockChatStream::default()
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

/// The `TransformStream` of `BedrockChatLanguageModel.doStream`.
#[derive(Default)]
struct AmazonBedrockChatStream {
    ctx: StreamContext,
    request_id: Option<String>,
    response_timestamp: Option<aimux_core::Timestamp>,
    model_id: String,
    uses_json_tool: bool,
    extractor: Option<JsonObjectTextExtractor>,
    is_json_response_from_tool: bool,
    text_blocks: HashSet<usize>,
    reasoning_blocks: HashMap<usize, Option<String>>,
    block_counter: usize,
    /// Tool call state: block_index → (id, name, accumulated_json)
    tool_blocks: HashMap<usize, (String, String, String)>,
    final_usage: Usage,
    final_finish_reason: Option<FinishReason>,
    /// Provider metadata accumulated for the Finish chunk from `metadata`
    /// and `messageStop` events (findings #26, #27). Mirrors the TS
    /// `providerMetadata` / `stopSequence` accumulation in
    /// `amazon-bedrock-chat-language-model.ts` (`doStream`).
    finish_meta: serde_json::Map<String, serde_json::Value>,
    stop_sequence: Option<String>,
}

impl AmazonBedrockChatStream {
    fn content_block_start(&mut self, payload: &serde_json::Value, controller: &mut Controller) {
        let idx = payload
            .get("contentBlockIndex")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(self.block_counter as u64) as usize;

        // Check if this is a tool use block.
        if let Some(start) = payload.get("start") {
            if let Some(tool_use) = start.get("toolUse") {
                let name = tool_use
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let id = tool_use
                    .get("toolUseId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let id = normalize_tool_call_id(
                    &if id.is_empty() {
                        aimux_provider_utils::generate_id()
                    } else {
                        id
                    },
                    self.model_id.contains("mistral."),
                );
                if !(self.uses_json_tool && name == "json") {
                    controller.tool_input_start(id.clone(), name.clone());
                }
                self.tool_blocks.insert(idx, (id, name, String::new()));
            } else {
                // Text block.
                self.block_counter = idx + 1;
                let id = idx.to_string();
                self.text_blocks.insert(idx);
                controller.text_start(id);
            }
        } else {
            // Default: text block.
            let id = idx.to_string();
            self.text_blocks.insert(idx);
            controller.text_start(id);
        }
    }

    fn content_block_delta(&mut self, payload: &serde_json::Value, controller: &mut Controller) {
        let idx = payload
            .get("contentBlockIndex")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;

        let Some(delta) = payload.get("delta") else {
            return;
        };
        // Text delta
        if let Some(text) = delta.get("text").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            if self.text_blocks.insert(idx) {
                let id = idx.to_string();
                self.text_blocks.insert(idx);
                controller.text_start(id);
            }
            let delta = self
                .extractor
                .as_mut()
                .map(|extractor| extractor.process(text))
                .unwrap_or_else(|| text.to_string());
            if !delta.is_empty() {
                controller.text_delta(idx.to_string(), delta);
            }
        }
        // Tool use input delta
        if let Some(partial) = delta.get("toolUse").and_then(|t| t.get("input"))
            && let Some(partial_str) = partial.as_str()
            && let Some((id, name, acc)) = self.tool_blocks.get_mut(&idx)
            && !partial_str.is_empty()
        {
            acc.push_str(partial_str);
            if !(self.uses_json_tool && name == "json") {
                let id = id.clone();
                controller.tool_input_delta(id, partial_str.to_string());
            }
        }
        if let Some(rc) = delta.get("reasoningContent") {
            if !["text", "signature", "data", "redactedContent"]
                .iter()
                .any(|key| {
                    rc.get(key)
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !s.is_empty())
                })
            {
                return;
            }
            let id = idx.to_string();
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.reasoning_blocks.entry(idx)
            {
                entry.insert(None);
                controller.reasoning_start(id.clone());
            }
            if let Some(text) = rc.get("text").and_then(|v| v.as_str()) {
                controller.reasoning_delta(id.clone(), text.to_string());
            } else if let Some(sig) = rc
                .get("signature")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                controller.enqueue(StreamPart::ReasoningDelta {
                    id: id.clone(),
                    delta: String::new(),
                    provider_metadata: Some(options::metadata(json!({"signature": sig}))),
                });
            } else if let Some(data) = rc
                .get("data")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                controller.enqueue(StreamPart::ReasoningDelta {
                    id: id.clone(),
                    delta: String::new(),
                    provider_metadata: Some(options::metadata(json!({"redactedData": data}))),
                });
            } else if let Some(data) = rc.get("redactedContent").and_then(|v| v.as_str()) {
                self.reasoning_blocks
                    .entry(idx)
                    .or_default()
                    .get_or_insert_with(String::new)
                    .push_str(data);
            }
        }
    }

    fn content_block_stop(&mut self, payload: &serde_json::Value, controller: &mut Controller) {
        let idx = payload
            .get("contentBlockIndex")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;

        if let Some((id, name, acc)) = self.tool_blocks.remove(&idx) {
            if self.uses_json_tool && name == "json" {
                self.is_json_response_from_tool = true;
                let id = idx.to_string();
                controller.text_start(id.clone());
                controller.text_delta(id.clone(), acc);
                controller.text_end(id);
            } else {
                controller.tool_input_end(id.clone());
                // Empty input normalizes to "{}" per the upstream
                // provider.
                let input = if acc.is_empty() {
                    "{}".to_string()
                } else {
                    acc
                };
                controller.enqueue(StreamPart::ToolCall(RawToolCall {
                    tool_call_id: normalize_tool_call_id(&id, self.model_id.contains("mistral.")),
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                }));
            }
        } else if let Some(redacted) = self.reasoning_blocks.remove(&idx) {
            controller.enqueue(StreamPart::ReasoningEnd {
                id: idx.to_string(),
                provider_metadata: reasoning_redacted_meta(redacted),
            });
        } else if self.text_blocks.remove(&idx) {
            controller.text_end(idx.to_string());
        }
    }

    fn message_stop(&mut self, payload: &serde_json::Value) {
        if let Some(reason) = payload.get("stopReason").and_then(|v| v.as_str()) {
            let mut finish_reason = map_finish_reason(reason);
            if self.is_json_response_from_tool
                && finish_reason.unified == FinishReasonUnified::ToolCalls
            {
                finish_reason.unified = FinishReasonUnified::Stop;
            }
            self.final_finish_reason = Some(finish_reason);
        }
        // #26: surface which stop sequence sentinel fired
        // (additionalModelResponseFields.delta.stop_sequence), if any.
        if let Some(seq) = payload
            .get("additionalModelResponseFields")
            .and_then(|f| f.get("delta"))
            .and_then(|d| d.get("stop_sequence"))
            .and_then(|v| v.as_str())
        {
            self.stop_sequence = Some(seq.to_string());
        }
    }

    fn metadata(&mut self, payload: &serde_json::Value, controller: &mut Controller) {
        if let Some(usage) = payload.get("usage") {
            let bedrock_usage: super::api_types::BedrockUsage =
                match serde_json::from_value(usage.clone()) {
                    Ok(usage) => usage,
                    Err(error) => {
                        self.final_finish_reason = Some(FINISH_ERROR);
                        controller.enqueue(StreamPart::Error {
                            error: AiMuxError::InvalidResponseData {
                                message: format!("{error}; usage: {usage}"),
                                cause: Some(ErrorCause::source(&error)),
                            },
                        });
                        return;
                    }
                };
            self.final_usage = convert_usage(Some(&bedrock_usage));
            if let Some(cache_usage) = cache_usage_metadata(Some(&bedrock_usage)) {
                self.finish_meta.insert("usage".into(), cache_usage);
            }
        }
        // #27: surface guardrails trace, performanceConfig, and
        // serviceTier into the Finish chunk's provider_metadata
        // (mirrors TS `doStream` metadata handling).
        if let Some(trace) = payload.get("trace") {
            self.finish_meta.insert("trace".to_string(), trace.clone());
        }
        if let Some(pc) = payload.get("performanceConfig") {
            self.finish_meta
                .insert("performanceConfig".to_string(), pc.clone());
        }
        if let Some(st) = payload.get("serviceTier") {
            self.finish_meta
                .insert("serviceTier".to_string(), st.clone());
        }
    }
}

impl Transformer for AmazonBedrockChatStream {
    type Input = Result<EventStreamMessage, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
        controller.enqueue(StreamPart::ResponseMetadata(ResponseMetadata {
            id: self.request_id.take(),
            timestamp: self.response_timestamp.take(),
            model_id: Some(self.model_id.clone()),
        }));
    }

    fn transform(
        &mut self,
        message: Result<EventStreamMessage, AiMuxError>,
        controller: &mut Controller,
    ) {
        let msg = match message {
            Ok(msg) => msg,
            Err(error) => {
                controller.error(error);
                return;
            }
        };
        let payload: serde_json::Value = match serde_json::from_str(&msg.data) {
            Ok(v) => v,
            Err(error) => {
                self.final_finish_reason = Some(FINISH_ERROR);
                controller.enqueue(StreamPart::Error {
                    error: AiMuxError::from(error),
                });
                return;
            }
        };
        self.ctx.raw(
            &json!({ msg.event_type.clone(): payload.clone() }),
            controller,
        );
        if msg.message_type != "event" || msg.event_type.ends_with("Exception") {
            self.final_finish_reason = Some(FINISH_ERROR);
            let (status_code, is_retryable) = match msg.event_type.as_str() {
                "internalServerException" | "InternalServerException" => (Some(500), true),
                "modelStreamErrorException" | "ModelStreamErrorException" => (Some(424), true),
                "serviceUnavailableException" | "ServiceUnavailableException" => (Some(503), true),
                "throttlingException" | "ThrottlingException" => (Some(429), true),
                "validationException" | "ValidationException" => (Some(400), false),
                _ => (None, false),
            };
            let message = payload
                .get("message")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Amazon Bedrock stream failed with {}", msg.event_type));
            let data = json!({ msg.event_type.clone(): payload });
            controller.enqueue(StreamPart::Error {
                error: aimux_provider_utils::create_provider_stream_error(
                    aimux_provider_utils::ProviderStreamError {
                        message,
                        provider_code: Some(msg.event_type),
                        status_code,
                        is_retryable: Some(is_retryable),
                        data: data.clone(),
                    },
                    &data,
                    self.ctx.url.clone(),
                    self.ctx.request_body.clone(),
                    self.ctx.response_headers.clone(),
                ),
            });
            return;
        }
        match msg.event_type.as_str() {
            "messageStart" => {
                // Nothing to emit; response metadata already sent.
            }
            "contentBlockStart" => self.content_block_start(&payload, controller),
            "contentBlockDelta" => self.content_block_delta(&payload, controller),
            "contentBlockStop" => self.content_block_stop(&payload, controller),
            "messageStop" => self.message_stop(&payload),
            "metadata" => self.metadata(&payload, controller),
            _ => {}
        }
    }

    fn flush(mut self, controller: &mut Controller) {
        if self.ctx.fail_without_terminal(
            self.final_finish_reason.is_some(),
            "a messageStop event",
            controller,
        ) {
            return;
        }
        // #26: merge the stop sentinel into the metadata payload, then build
        // the Finish provider_metadata under `amazonBedrock` (mirrors the
        // TS `doStream` flush handler).
        if self.is_json_response_from_tool {
            self.finish_meta
                .insert("isJsonResponseFromTool".into(), json!(true));
            self.finish_meta
                .insert("stopSequence".into(), serde_json::Value::Null);
        }
        if let Some(seq) = self.stop_sequence {
            self.finish_meta
                .insert("stopSequence".to_string(), serde_json::Value::String(seq));
        }
        let provider_metadata = if self.finish_meta.is_empty() {
            None
        } else {
            let payload = serde_json::Value::Object(self.finish_meta);
            Some(options::metadata(payload))
        };

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FINISH_OTHER),
            usage: self.final_usage,
            provider_metadata,
        });
    }
}

/// Attach accumulated encrypted reasoning once at the end of the block.
fn reasoning_redacted_meta(sig: Option<String>) -> Option<ProviderMetadata> {
    sig.map(|s| options::metadata(json!({ "redactedContent": s })))
}

/// Extract `GenerateContent` items from a non-streaming content block.
///
/// Bedrock content blocks are field-tagged: each block carries exactly one of
/// `text`, `toolUse`, or `reasoningContent`. Empty `text` blocks are preserved
/// (matching the TS SDK) so that empty text between reasoning blocks survives.
/// `reasoningContent.reasoningText` yields a `Reasoning` item whose
/// `provider_metadata` carries the `signature` under `amazonBedrock` (or
/// `None` when no signature is present).
/// `reasoningContent.redactedReasoning` yields a `Reasoning` item with empty
/// text and `redactedData` under the same key.
fn extract_content(
    block: &BedrockContentBlock,
    content: &mut Vec<GenerateContent>,
    model_id: &str,
) {
    if let Some(text) = &block.text {
        content.push(GenerateContent::Text {
            text: text.clone(),
            provider_metadata: None,
        });
    } else if let Some(citation) = &block.citations_content
        && let Some(parts) = citation.get("content").and_then(|v| v.as_array())
    {
        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                content.push(GenerateContent::Text {
                    text: text.into(),
                    provider_metadata: None,
                });
            }
        }
    }
    if let Some(tool_use) = &block.tool_use {
        content.push(GenerateContent::ToolCall(RawToolCall {
            tool_call_id: normalize_tool_call_id(
                &if tool_use.tool_use_id.is_empty() {
                    aimux_provider_utils::generate_id()
                } else {
                    tool_use.tool_use_id.clone()
                },
                model_id.contains("mistral."),
            ),
            tool_name: tool_use.name.clone(),
            input: if tool_use.input.is_null() {
                "{}".into()
            } else {
                tool_use.input.to_string()
            },
            provider_executed: None,
            dynamic: None,
            provider_metadata: None,
        }));
    }
    if let Some(rc) = &block.reasoning_content {
        if let Some(rt) = rc.get("reasoningText") {
            let text = rt
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let provider_metadata = rt
                .get("signature")
                .and_then(|v| v.as_str())
                .filter(|sig| !sig.is_empty())
                .map(|sig| options::metadata(json!({ "signature": sig })));
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text,
                provider_metadata,
            }));
        } else if let Some(redacted) = rc.get("redactedContent") {
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text: String::new(),
                provider_metadata: Some(options::metadata(json!({"redactedContent": redacted}))),
            }));
        } else if let Some(rr) = rc.get("redactedReasoning") {
            let data = rr
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let provider_metadata = Some(options::metadata(json!({ "redactedData": data })));
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text: String::new(),
                provider_metadata,
            }));
        }
    }
}

fn cache_usage_metadata(
    usage: Option<&super::api_types::BedrockUsage>,
) -> Option<serde_json::Value> {
    let usage = usage?;
    let mut payload = serde_json::Map::new();
    if let Some(tokens) = usage.cache_write_input_tokens {
        payload.insert("cacheWriteInputTokens".into(), json!(tokens));
    }
    if let Some(details) = &usage.cache_details {
        payload.insert(
            "cacheDetails".into(),
            serde_json::to_value(details).unwrap_or_default(),
        );
    }
    (!payload.is_empty()).then_some(serde_json::Value::Object(payload))
}

fn response_provider_metadata(data: &BedrockConverseResponse) -> Option<ProviderMetadata> {
    if data.trace.is_none()
        && data.usage.is_none()
        && data.performance_config.is_none()
        && data.service_tier.is_none()
        && data
            .additional_model_response_fields
            .as_ref()
            .and_then(|v| v.get("delta"))
            .and_then(|v| v.get("stop_sequence"))
            .is_none()
    {
        return None;
    }
    let mut payload = serde_json::Map::new();
    for (key, value) in [
        ("trace", &data.trace),
        ("performanceConfig", &data.performance_config),
        ("serviceTier", &data.service_tier),
    ] {
        if let Some(value) = value {
            if key == "trace" && !value.is_object() {
                continue;
            }
            payload.insert(key.into(), value.clone());
        }
    }
    if let Some(usage) = cache_usage_metadata(data.usage.as_ref()) {
        payload.insert("usage".into(), usage);
    }
    payload.insert(
        "stopSequence".into(),
        data.additional_model_response_fields
            .as_ref()
            .and_then(|v| v.get("delta"))
            .and_then(|v| v.get("stop_sequence"))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    );
    Some(options::metadata(serde_json::Value::Object(payload)))
}

#[derive(Default)]
struct JsonObjectTextExtractor {
    started: bool,
    completed: bool,
    depth: usize,
    in_string: bool,
    escaped: bool,
}

impl JsonObjectTextExtractor {
    fn process(&mut self, text: &str) -> String {
        let mut result = String::new();
        for character in text.chars() {
            if self.completed {
                break;
            }
            if !self.started {
                if character != '{' {
                    continue;
                }
                self.started = true;
                self.depth = 1;
                result.push(character);
                continue;
            }
            result.push(character);
            if self.escaped {
                self.escaped = false;
                continue;
            }
            if character == '\\' && self.in_string {
                self.escaped = true;
                continue;
            }
            if character == '"' {
                self.in_string = !self.in_string;
                continue;
            }
            if self.in_string {
                continue;
            }
            if character == '{' {
                self.depth += 1;
            } else if character == '}' {
                self.depth -= 1;
                self.completed = self.depth == 0;
            }
        }
        result
    }
}

// ── Request body ────────────────────────────────────────────────────────────

// The request capabilities come from Anthropic's getModelCapabilities, not
// Bedrock's separate strict-tool support list. Unknown newer model IDs assume
// adaptive thinking and reject sampling; legacy IDs retain their defaults.
/// `amazonBedrockReasoningEffortMap` of amazon-bedrock-chat-language-model.ts.
const AMAZON_BEDROCK_REASONING_EFFORT_MAP: &[(aimux_core::types::ReasoningEffort, &str)] = &[
    (aimux_core::types::ReasoningEffort::Minimal, "low"),
    (aimux_core::types::ReasoningEffort::Low, "low"),
    (aimux_core::types::ReasoningEffort::Medium, "medium"),
    (aimux_core::types::ReasoningEffort::High, "high"),
    (aimux_core::types::ReasoningEffort::Xhigh, "max"),
];

fn reasoning_capabilities(model_id: &str) -> (u32, bool, bool, bool) {
    let Some((_, model)) = model_id.split_once("claude-") else {
        return (4096, false, false, false);
    };
    if ["opus-5", "fable-5", "sonnet-5", "opus-4-8", "opus-4-7"]
        .iter()
        .any(|id| model.starts_with(id))
    {
        (128000, true, true, true)
    } else if ["sonnet-4-6", "opus-4-6"]
        .iter()
        .any(|id| model.starts_with(id))
    {
        (128000, true, false, true)
    } else if ["sonnet-4-5", "opus-4-5", "haiku-4-5"]
        .iter()
        .any(|id| model.starts_with(id))
    {
        (64000, false, false, true)
    } else if model.starts_with("opus-4-1") {
        (32000, false, false, true)
    } else if model.starts_with("sonnet-4-") || model.starts_with("sonnet-4@") {
        (64000, false, false, false)
    } else if model.starts_with("opus-4-") || model.starts_with("opus-4@") {
        (32000, false, false, false)
    } else {
        let two = model.strip_prefix('v').unwrap_or(model);
        let legacy = model == "instant"
            || model.starts_with("instant-")
            || two == "2"
            || two.starts_with("2-")
            || two.starts_with("2.")
            || two.starts_with("2:")
            || model == "3"
            || model.starts_with("3-")
            || model.starts_with("3.");
        if legacy {
            (4096, false, false, false)
        } else {
            (128000, true, true, true)
        }
    }
}

/// Build a checked Converse request, preserving upstream warnings.
///
/// # Errors
///
/// A prompt or tool definition the Converse API cannot express.
pub fn build_request_body_checked(
    model_id: &str,
    call: &CallOptions,
    model_family: Option<&str>,
) -> Result<(Value, Vec<aimux_core::types::Warning>, bool, bool), aimux_core::AiMuxError> {
    use aimux_core::types::{ReasoningEffort, Warning};
    let mut warnings = Vec::new();
    let mut compatibility = Vec::new();
    let mut warn = |feature: &str, details: Option<&str>| {
        warnings.push(Warning::Unsupported {
            feature: feature.into(),
            details: details.map(str::to_owned),
        })
    };
    let bedrock = options::read(call.provider_options.as_ref());
    validate_options(bedrock)?;
    let mut reasoning = bedrock
        .and_then(|b| b.get("reasoningConfig"))
        .cloned()
        .unwrap_or(json!({}));
    let anthropic = model_family == Some("anthropic")
        || model_id.contains("anthropic")
        || (model_id.contains(":application-inference-profile/")
            && reasoning.get("budgetTokens").is_some());
    let openai = model_id
        .split('.')
        .any(|p| p == options::OPENAI_MODEL_FAMILY);
    let oss = openai && model_id.contains("-oss-");
    let nova = model_id.contains("amazon.nova-2-lite-v1:0");
    let (max_reasoning_tokens, adaptive, rejects_sampling, supports_structured_output) =
        reasoning_capabilities(model_id);
    if let Some(effort) = &call.reasoning
        && effort.is_custom()
    {
        if anthropic && matches!(effort, ReasoningEffort::None) {
            reasoning = json!({"type":"disabled"});
        } else if !matches!(effort, ReasoningEffort::None)
            && (anthropic
                || openai
                || nova
                || bedrock.is_some_and(|b| b.contains_key("reasoningConfig")))
        {
            let mut derived = if anthropic && !adaptive {
                let budget_tokens = aimux_provider_utils::map_reasoning_to_provider_budget(
                    *effort,
                    max_reasoning_tokens,
                    max_reasoning_tokens,
                    aimux_provider_utils::DEFAULT_MIN_REASONING_BUDGET,
                    aimux_provider_utils::DEFAULT_REASONING_BUDGET_PERCENTAGES,
                    &mut compatibility,
                );
                json!({"type":"enabled","budgetTokens":budget_tokens})
            } else {
                let level = aimux_provider_utils::map_reasoning_to_provider_effort(
                    *effort,
                    AMAZON_BEDROCK_REASONING_EFFORT_MAP,
                    &mut compatibility,
                );
                if anthropic {
                    json!({"type":"adaptive","maxReasoningEffort":level})
                } else {
                    json!({"maxReasoningEffort":level})
                }
            };
            if nova {
                derived["type"] = json!("enabled");
            }
            if let Some(explicit) = reasoning.as_object() {
                derived.as_object_mut().unwrap().extend(explicit.clone());
            }
            reasoning = derived;
        } else if !matches!(effort, ReasoningEffort::None) {
            warn(
                "reasoning",
                Some(
                    "Portable reasoning is not supported for this model and will be ignored. If the model supports a provider-specific reasoning configuration, use providerOptions.amazonBedrock.reasoningConfig.",
                ),
            );
        }
        if reasoning["type"] == "disabled" {
            reasoning.as_object_mut().unwrap().remove("budgetTokens");
            reasoning
                .as_object_mut()
                .unwrap()
                .remove("maxReasoningEffort");
        }
    }
    let thinking = anthropic && matches!(reasoning["type"].as_str(), Some("enabled" | "adaptive"));
    let rejects_forced = anthropic
        && (model_id.contains("sonnet-5-5")
            || model_id.contains("opus-5-5")
            || model_id.contains("fable-5-1"));
    let mut inference = serde_json::Map::new();
    if let Some(max) = call.max_output_tokens {
        inference.insert("maxTokens".into(), json!(max));
    }
    let (mut temperature, mut top_p, mut top_k) = (call.temperature, call.top_p, call.top_k);
    if rejects_sampling {
        // amazon-bedrock-chat-language-model.ts:204-230: temperature, topK, topP.
        for (feature, given) in [
            ("temperature", temperature.take().is_some()),
            ("topK", top_k.take().is_some()),
            ("topP", top_p.take().is_some()),
        ] {
            if given {
                warn(
                    feature,
                    Some(&format!(
                        "{feature} is not supported by {model_id} and will be ignored"
                    )),
                );
            }
        }
    }
    if let Some(temp) = temperature {
        if thinking || (openai && !oss) {
            let details = if thinking {
                "temperature is not supported when thinking is enabled"
            } else {
                "temperature is not supported by this OpenAI model on the Converse API"
            };
            warn("temperature", Some(details));
        } else {
            let temp = if !openai || oss {
                if temp > 1.0 {
                    warn(
                        "temperature",
                        Some(&format!(
                            "{temp} exceeds bedrock maximum of 1.0. clamped to 1.0"
                        )),
                    );
                    1.0
                } else if temp < 0.0 {
                    warn(
                        "temperature",
                        Some(&format!(
                            "{temp} is below bedrock minimum of 0. clamped to 0"
                        )),
                    );
                    0.0
                } else {
                    temp
                }
            } else {
                temp
            };
            inference.insert("temperature".into(), json!(temp));
        }
    }
    if let Some(p) = top_p {
        if thinking || (openai && !oss) {
            let details = if thinking {
                "topP is not supported when thinking is enabled"
            } else {
                "topP is not supported by this OpenAI model on the Converse API"
            };
            warn("topP", Some(details));
        } else {
            inference.insert("topP".into(), json!(p));
        }
    }
    if let Some(k) = top_k {
        if thinking {
            warn(
                "topK",
                Some("topK is not supported when thinking is enabled"),
            );
        } else {
            inference.insert("topK".into(), json!(k));
        }
    }
    if let Some(stop) = &call.stop_sequences {
        if openai {
            warn(
                "stopSequences",
                Some("stopSequences is not supported by this OpenAI model on the Converse API"),
            );
        } else {
            inference.insert("stopSequences".into(), json!(stop));
        }
    }
    for (feature, present) in [
        ("frequencyPenalty", call.frequency_penalty.is_some()),
        ("presencePenalty", call.presence_penalty.is_some()),
        ("seed", call.seed.is_some()),
    ] {
        if present {
            warn(feature, None);
        }
    }
    let mut fields = bedrock
        .and_then(|b| b.get("additionalModelRequestFields"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if thinking && reasoning["type"] == "enabled" {
        if let Some(budget) = reasoning["budgetTokens"].as_u64() {
            let max = inference
                .get("maxTokens")
                .and_then(Value::as_u64)
                .unwrap_or(4096);
            inference.insert("maxTokens".into(), json!(max.saturating_add(budget)));
            fields.insert(
                "thinking".into(),
                json!({"type":"enabled","budget_tokens":budget}),
            );
        }
    } else if thinking && reasoning["type"] == "adaptive" {
        let mut config = json!({"type":"adaptive"});
        if let Some(display) = reasoning.get("display") {
            config["display"] = display.clone();
        }
        fields.insert("thinking".into(), config);
    } else if !anthropic {
        if reasoning.get("budgetTokens").is_some() {
            warn(
                "budgetTokens",
                Some(
                    "budgetTokens applies only to Anthropic models on Bedrock and will be ignored for this model.",
                ),
            );
        }
        if reasoning["type"] == "adaptive" {
            warn(
                "adaptive thinking",
                Some("adaptive thinking type applies only to Anthropic models on Bedrock."),
            );
        }
    }
    if let Some(effort) = reasoning.get("maxReasoningEffort") {
        let key = if anthropic {
            "output_config"
        } else if openai && !oss {
            "reasoning"
        } else {
            "reasoningConfig"
        };
        if oss {
            fields.insert("reasoning_effort".into(), effort.clone());
        } else {
            let mut config = fields
                .get(key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            config.insert(
                if anthropic || openai {
                    "effort"
                } else {
                    "maxReasoningEffort"
                }
                .into(),
                effort.clone(),
            );
            if !anthropic && !openai {
                if let Some(kind) = reasoning.get("type").filter(|t| *t != "adaptive") {
                    config.insert("type".into(), kind.clone());
                }
                if let Some(budget) = reasoning
                    .get("budgetTokens")
                    .filter(|_| reasoning["type"] == "enabled")
                {
                    config.insert("budgetTokens".into(), budget.clone());
                }
            }
            fields.insert(key.into(), Value::Object(config));
        }
        if nova
            && reasoning["type"] == "enabled"
            && matches!(effort.as_str(), Some("high" | "xhigh" | "max"))
            && inference.remove("maxTokens").is_some()
        {
            warn(
                "maxOutputTokens",
                Some(&format!(
                    "maxOutputTokens is not supported by {model_id} when high reasoning is enabled and will be ignored"
                )),
            );
        }
    }
    if let Some(betas) = bedrock.and_then(|b| b.get("anthropicBeta")) {
        fields.insert("anthropic_beta".into(), betas.clone());
    }
    let anthropic_options = call
        .provider_options
        .as_ref()
        .and_then(|p| p.get("anthropic"));
    if let Some(value) = anthropic_options.and_then(|p| p.get("structuredOutputMode"))
        && !value
            .as_str()
            .is_some_and(|v| ["outputFormat", "jsonTool", "auto"].contains(&v))
    {
        return Err(aimux_core::AiMuxError::invalid_argument(
            "Invalid Anthropic structuredOutputMode",
        ));
    }
    if anthropic_options
        .and_then(|p| p.get("disableParallelToolUse"))
        .is_some_and(|v| !v.is_boolean())
    {
        return Err(aimux_core::AiMuxError::invalid_argument(
            "Invalid Anthropic disableParallelToolUse",
        ));
    }
    let structured_mode = bedrock
        .and_then(|b| b.get("structuredOutputMode"))
        .or_else(|| anthropic_options.and_then(|p| p.get("structuredOutputMode")))
        .and_then(Value::as_str)
        .unwrap_or("auto");
    if structured_mode == "jsonTool"
        && let Some(output_config) = fields
            .get_mut("output_config")
            .and_then(Value::as_object_mut)
    {
        output_config.remove("format");
        if output_config.is_empty() {
            fields.remove("output_config");
        }
    }
    let schema = match &call.response_format {
        Some(ResponseFormat::Json { schema, .. }) => schema.as_ref(),
        _ => None,
    };
    let supports_native = supports_native_structured_output(model_id);
    let uses_native = anthropic
        && schema.is_some()
        && (structured_mode == "outputFormat"
            || (structured_mode == "auto"
                && supports_native
                && (supports_structured_output || thinking || model_family == Some("anthropic"))));
    let uses_json_instruction = !uses_native
        && anthropic
        && schema.is_some()
        && (rejects_forced
            || (structured_mode != "jsonTool"
                && !supports_strict_tools(model_id)
                && call.tools.as_ref().is_some_and(|tools| !tools.is_empty())));
    let uses_json_tool = schema.is_some() && !uses_native && !uses_json_instruction;
    if uses_native {
        let output_config = fields.entry("output_config").or_insert_with(|| json!({}));
        if !output_config.is_object() {
            *output_config = json!({});
        }
        output_config["format"] = json!({"type":"json_schema", "schema":crate::anthropic::sanitize_json_schema::sanitize_json_schema(schema.unwrap())});
    }
    let mut all_tools = call.tools.clone().unwrap_or_default();
    if uses_json_tool {
        all_tools.push(Tool::Function(
            FunctionTool::new("json", schema.unwrap().clone())
                .with_description("Respond with a JSON object."),
        ));
    }
    let tool_choice = if uses_json_tool {
        Some(LanguageModelV4ToolChoice::Required)
    } else {
        call.tool_choice.clone()
    };
    let disable_parallel = anthropic_options
        .and_then(|p| p.get("disableParallelToolUse"))
        .and_then(Value::as_bool)
        == Some(true);
    let mut provider_tools = super::prepare_tools::prepare_provider_tools(
        Some(&all_tools),
        tool_choice.as_ref(),
        anthropic,
        disable_parallel,
        rejects_forced,
    )?;
    for warning in provider_tools.warnings.drain(..) {
        match warning {
            Warning::Unsupported { feature, details } => warn(&feature, details.as_deref()),
            other => compatibility.push(other),
        }
    }
    let tools: Option<Vec<FunctionTool>> = Some(
        all_tools
            .iter()
            .filter_map(|tool| match tool {
                Tool::Function(f) => Some(f.clone()),
                _ => None,
            })
            .collect(),
    );
    let function_choice = if provider_tools.using_anthropic_tools
        && matches!(tool_choice, Some(LanguageModelV4ToolChoice::None))
    {
        Some(LanguageModelV4ToolChoice::Auto)
    } else {
        tool_choice.clone()
    };
    let mut tool_config = prepare_tools(&tools, function_choice.as_ref(), model_id);
    if !provider_tools.tools.is_empty() {
        let mut combined = provider_tools.tools;
        if let Some(functions) = tool_config.get("tools").and_then(Value::as_array) {
            combined.extend(functions.clone());
        }
        tool_config["tools"] = json!(combined);
    }
    let has_additional_tools = provider_tools.tool_choice.is_some();
    if provider_tools.using_anthropic_tools {
        tool_config.as_object_mut().unwrap().remove("toolChoice");
        if let Some(choice) = provider_tools.tool_choice {
            fields.insert("tool_choice".into(), choice);
        }
    }
    if !provider_tools.betas.is_empty() {
        let mut betas = bedrock
            .and_then(|b| b.get("anthropicBeta"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        betas.extend(provider_tools.betas.into_iter().map(Value::String));
        fields.insert("anthropic_beta".into(), json!(betas));
    }
    if !provider_tools.using_anthropic_tools
        && rejects_forced
        && matches!(tool_choice, Some(LanguageModelV4ToolChoice::Required | LanguageModelV4ToolChoice::Tool { .. }))
        && all_tools.iter().any(|tool| !matches!(tool, Tool::Provider(provider) if matches!(provider.id.as_str(), "anthropic.web_search_20250305" | "anthropic.web_search_20260318" | "anthropic.web_fetch_20260318")))
    {
        if tool_config.get("tools").is_some() { tool_config["toolChoice"] = json!({"auto":{}}); }
        let details = match &tool_choice {
            Some(LanguageModelV4ToolChoice::Tool {tool_name}) => format!("toolChoice 'tool' is not supported by this model because it rejects forced tool use. Only the '{tool_name}' tool is sent with 'auto' tool choice. Instruct the model to use the tool in the prompt and verify that a tool call was made."),
            _ => "toolChoice 'required' is not supported by this model because it rejects forced tool use. Using 'auto' instead. Instruct the model to use a tool in the prompt and verify that a tool call was made.".into(),
        };
        warn("toolChoice", Some(&details));
    }
    if let Some(tools) = &tools {
        for tool in tools.iter().filter(|tool| !matches!(&function_choice, Some(LanguageModelV4ToolChoice::Tool { tool_name }) if tool_name != &tool.name)) {
            if let Some(strict) = tool.strict
                && !supports_strict_tools(model_id)
            {
                warn(
                    "strict",
                    Some(&format!(
                        "Tool '{}' has strict: {}, but strict mode is not supported by this model on Amazon Bedrock. The strict property will be ignored.",
                        tool.name, strict
                    )),
                );
            } else if tool.strict == Some(true) && !is_strict_schema_compatible(&tool.input_schema)
            {
                warn(
                    "strict",
                    Some(&format!(
                        "Tool '{}' has strict: true, but Amazon Bedrock requires every object in a strict tool schema to set additionalProperties: false. The strict property will be ignored.",
                        tool.name
                    )),
                );
            }
        }
    }
    if !provider_tools.using_anthropic_tools
        && anthropic
        && disable_parallel
        && tool_config.get("tools").is_some()
    {
        let choice = if rejects_forced {
            json!({"type":"auto","disable_parallel_tool_use":true})
        } else {
            match &tool_choice {
                Some(LanguageModelV4ToolChoice::Required) => {
                    json!({"type":"any","disable_parallel_tool_use":true})
                }
                Some(LanguageModelV4ToolChoice::Tool { tool_name }) => {
                    json!({"type":"tool","name":tool_name,"disable_parallel_tool_use":true})
                }
                _ => json!({"type":"auto","disable_parallel_tool_use":true}),
            }
        };
        fields.insert("tool_choice".into(), choice);
        tool_config.as_object_mut().unwrap().remove("toolChoice");
    }
    let mut prompt = call.prompt.clone();
    if tool_config.get("tools").is_none() && !has_additional_tools {
        let has_tools = prompt.iter().any(|message| match message {
            LanguageModelMessage::Assistant { content, .. } => content.iter().any(|part| {
                matches!(
                    part,
                    AssistantPart::ToolCall(_) | AssistantPart::ToolResult(_)
                )
            }),
            LanguageModelMessage::Tool { content, .. } => !content.is_empty(),
            _ => false,
        });
        if has_tools {
            for message in &mut prompt {
                match message {
                    LanguageModelMessage::Assistant { content, .. } => content.retain(|part| {
                        !matches!(
                            part,
                            AssistantPart::ToolCall(_) | AssistantPart::ToolResult(_)
                        )
                    }),
                    LanguageModelMessage::Tool { content, .. } => content.clear(),
                    _ => {}
                }
            }
            prompt.retain(|message| match message {
                LanguageModelMessage::System { .. } => true,
                LanguageModelMessage::User { content, .. } => !content.is_empty(),
                LanguageModelMessage::Assistant { content, .. } => !content.is_empty(),
                LanguageModelMessage::Tool { content, .. } => !content.is_empty(),
            });
            warn(
                "toolContent",
                Some(
                    "Tool calls and results removed from conversation because Bedrock does not support tool content without active tools.",
                ),
            );
        }
    }
    if uses_json_instruction {
        // @ai-sdk/amazon-bedrock@5.0.100 amazon-bedrock-chat-language-model.ts:614-621
        prompt = inject_json_instruction_into_messages(
            &prompt,
            schema,
            None,
            Some(
                "You MUST answer with only a JSON object that matches the JSON schema above. Do not wrap it in markdown fences or include any other text.",
            ),
        );
    }
    let (system, messages) = convert_prompt(&prompt, model_id.contains("mistral."))?;
    let mut body = json!({"messages":messages});
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !inference.is_empty() {
        body["inferenceConfig"] = Value::Object(inference);
    }
    if !fields.is_empty() {
        body["additionalModelRequestFields"] = Value::Object(fields);
    }
    if anthropic {
        body["additionalModelResponseFieldPaths"] = json!(["/delta/stop_sequence"]);
    }
    if let Some(tier) = bedrock.and_then(|b| b.get("serviceTier")) {
        body["serviceTier"] = json!({"type":tier});
    }
    if let Some(bedrock) = bedrock {
        for (key, value) in bedrock {
            if !matches!(
                key.as_str(),
                "reasoningConfig"
                    | "additionalModelRequestFields"
                    | "serviceTier"
                    | "structuredOutputMode"
            ) {
                body[key] = value.clone();
            }
        }
    }
    if tool_config.get("tools").is_some() {
        body["toolConfig"] = tool_config;
    }
    warnings.extend(compatibility);
    Ok((body, warnings, uses_json_instruction, uses_json_tool))
}
