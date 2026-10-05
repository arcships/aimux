//! Amazon Bedrock language model — implements `LanguageModel`.
//!
//! Uses the Bedrock Converse API (`/model/{model-id}/converse` and
//! `/model/{model-id}/converse-stream`), which provides a unified interface
//! across all Bedrock-backed models (Anthropic Claude, Meta Llama, Mistral,
//! etc.).

use aimux_core::tool::RawToolCall;
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ProviderMetadata, ResponseMetadata, Usage,
};

use serde_json::json;

use aimux_provider_utils::HttpBody;

use super::convert::{
    build_request_body_checked, convert_usage, map_finish_reason, normalize_tool_call_id,
};
use super::options;
use super::types::{BedrockContentBlock, BedrockConverseResponse};
use crate::shared::EndpointConfig;

fn bedrock_event_stream_response_handler()
-> aimux_provider_utils::ResponseHandler<BoxStream<'static, Result<Bytes, AiMuxError>>> {
    aimux_provider_utils::ResponseHandler::new(|input| async move {
        let headers = aimux_provider_utils::extract_response_headers::extract_response_headers(
            input.response.headers(),
        );
        let output_headers = headers.clone();
        let url = input.url;
        let request_body_values = input.request_body_values;
        let signal = input.abort_signal;
        let stream = input.response.bytes_stream().map(move |result| {
            result.map_err(|error| {
                AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                    response_headers: Some(headers.clone()),
                    is_retryable: true,
                    ..aimux_core::ApiCallError::new(
                        error.to_string(),
                        url.clone(),
                        request_body_values.clone(),
                    )
                }))
            })
        });
        let value: BoxStream<'static, Result<Bytes, AiMuxError>> = match signal {
            Some(signal) => Box::pin(async_stream::stream! {
                futures::pin_mut!(stream);
                loop {
                    tokio::select! {
                        biased;
                        () = signal.cancelled() => {
                            yield Err(AiMuxError::from_abort_signal(&signal));
                            break;
                        }
                        item = stream.next() => match item {
                            Some(item) => yield item,
                            None => break,
                        }
                    }
                }
            }),
            None => Box::pin(stream),
        };
        Ok(aimux_provider_utils::ResponseHandlerOutput {
            value,
            raw_value: None,
            response_headers: output_headers,
        })
    })
    .streaming()
}

/// An Amazon Bedrock language model (e.g. `anthropic.claude-3-5-sonnet-20240620-v1:0`).
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct BedrockModel {
    model_id: String,
    config: EndpointConfig,
    model_family: Option<String>,
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
            super::encode_model_id(&self.model_id),
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

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let (body, warnings) =
            build_request_body_checked(&self.model_id, options, self.model_family.as_deref())?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
        let body = exchange.transform_body(body);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = exchange.url(&self.path(false));
        let resp = aimux_provider_utils::post_to_api(
            exchange.request(url, options),
            HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),
            aimux_provider_utils::create_json_response_handler(),
            super::bedrock_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let raw_data = resp.raw_value;
        let data: BedrockConverseResponse = resp.value;

        // Extract content from response.output.message.content
        let mut content = Vec::new();
        if let Some(output) = &data.output
            && let Some(message) = &output.message
        {
            for block in &message.content {
                extract_content(block, &mut content, &self.model_id);
            }
        }

        let finish_reason = data
            .stop_reason
            .as_deref()
            .map(map_finish_reason)
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });

        let mut usage = convert_usage(data.usage.as_ref());
        usage.raw = raw_data
            .as_ref()
            .and_then(|raw| raw.get("usage"))
            .filter(|v| !v.is_null())
            .cloned();

        let request_id = response_headers
            .get("x-amzn-requestid")
            .cloned()
            .or_else(|| response_headers.get("x-amzn-request-id").cloned());

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: response_provider_metadata(&data),
            response: Some(aimux_core::shared::ResponseInfo {
                id: request_id,
                timestamp: response_headers.get("date").cloned(),
                model_id: Some(self.model_id.clone()),
                headers: Some(response_headers),
                body: raw_data,
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let (body, warnings) =
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
            super::bedrock_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let body_stream = resp.value;
        let include_raw_chunks = options.include_raw_chunks.unwrap_or(false);
        let request_id = response_headers
            .get("x-amzn-requestid")
            .cloned()
            .or_else(|| response_headers.get("x-amzn-request-id").cloned());
        // Same source as the non-stream path: the response `Date` header
        // (RFC1123). Keeps stream/non-stream timestamps consistent.
        let response_timestamp = response_headers.get("date").cloned();

        let model_id = self.model_id.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            yield Ok(StreamPart::ResponseMetadata(ResponseMetadata {
                id: request_id,
                timestamp: response_timestamp,
                model_id: Some(model_id.clone()),
            }));

            let mut messages = super::event_stream::decode_stream(body_stream);

            let mut text_blocks = HashSet::new();
            let mut reasoning_blocks: HashMap<usize, Option<String>> = HashMap::new();
            let mut block_counter = 0usize;
            // Tool call state: block_index → (id, name, accumulated_json)
            let mut tool_blocks: HashMap<usize, (String, String, String)> = HashMap::new();
            let mut final_usage: Usage = Usage::default();
            let mut final_finish_reason: Option<FinishReason> = None;
            // Provider metadata accumulated for the Finish chunk from `metadata`
            // and `messageStop` events (findings #26, #27). Mirrors the TS
            // `providerMetadata` / `stopSequence` accumulation in
            // `amazon-bedrock-chat-language-model.ts` (`doStream`).
            let mut finish_meta: serde_json::Map<String, serde_json::Value> =
                serde_json::Map::new();
            let mut stop_sequence: Option<String> = None;

            while let Some(message) = messages.next().await {
                let msg = match message {
                    Ok(msg) => msg,
                    Err(error) => { yield Err(error); return; }
                };
                let payload: serde_json::Value = match serde_json::from_str(&msg.data) {
                    Ok(v) => v,
                    Err(error) => {
                        final_finish_reason = Some(FinishReason { unified: FinishReasonUnified::Error, raw: None });
                        yield Ok(StreamPart::Error { error: AiMuxError::from(error) });
                        continue;
                    }
                };
                if include_raw_chunks {
                    yield Ok(StreamPart::Raw { raw_value: json!({ msg.event_type.clone(): payload.clone() }) });
                }
                if msg.message_type != "event" || msg.event_type.ends_with("Exception") {
                    final_finish_reason = Some(FinishReason { unified: FinishReasonUnified::Error, raw: None });
                    let (status_code, is_retryable) = match msg.event_type.as_str() {
                        "internalServerException" | "InternalServerException" => (Some(500), true),
                        "modelStreamErrorException" | "ModelStreamErrorException" => (Some(424), true),
                        "serviceUnavailableException" | "ServiceUnavailableException" => (Some(503), true),
                        "throttlingException" | "ThrottlingException" => (Some(429), true),
                        "validationException" | "ValidationException" => (Some(400), false),
                        _ => (None, false),
                    };
                    let message = payload.get("message").and_then(|v| v.as_str()).map(str::to_string)
                        .unwrap_or_else(|| format!("Amazon Bedrock stream failed with {}", msg.event_type));
                    yield Ok(StreamPart::Error { error: AiMuxError::ApiCall(Box::new(aimux_core::ApiCallError {
                        status_code, is_retryable, provider_code: Some(msg.event_type.clone()),
                        data: Some(json!({msg.event_type.clone(): payload.clone()})),
                        ..aimux_core::ApiCallError::new(message, error_url.clone(), error_body.clone())
                    })) });
                    continue;
                }
                match msg.event_type.as_str() {
                    "messageStart" => {
                        // Nothing to emit; response metadata already sent.
                    }
                    "contentBlockStart" => {
                        let idx = payload
                            .get("contentBlockIndex")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(block_counter as u64) as usize;

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
                                let id = normalize_tool_call_id(&if id.is_empty() {
                                    aimux_provider_utils::generate_id()
                                } else {
                                    id
                                }, model_id.contains("mistral."));
                                yield Ok(StreamPart::ToolInputStart {
                                    id: id.clone(),
                                    tool_name: name.clone(),
                                    provider_executed: None,
                                    dynamic: None,
                                    title: None,
                                    provider_metadata: None,
                                });
                                tool_blocks.insert(idx, (id, name, String::new()));
                            } else {
                                // Text block.
                                block_counter = idx + 1;
                                let id = idx.to_string();
                                text_blocks.insert(idx);
                                yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                            }
                        } else {
                            // Default: text block.
                            let id = idx.to_string();
                            text_blocks.insert(idx);
                            yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                        }
                    }
                    "contentBlockDelta" => {
                        let idx = payload
                            .get("contentBlockIndex")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;

                        if let Some(delta) = payload.get("delta") {
                            // Text delta
                            if let Some(text) = delta.get("text").and_then(|v| v.as_str())
                                && !text.is_empty() {
                                    if text_blocks.insert(idx) {
                                        let id = idx.to_string();
                                        text_blocks.insert(idx);
                                        yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                                    }
                                    {
                                        let id = idx.to_string();
                                        yield Ok(StreamPart::TextDelta {
                                            id,
                                            delta: text.to_string(),
                                            provider_metadata: None,
                                        });
                                    }
                                }
                            // Tool use input delta
                            if let Some(partial) =
                                delta.get("toolUse").and_then(|t| t.get("input"))
                                && let Some(partial_str) = partial.as_str()
                                    && let Some((id, _name, acc)) = tool_blocks.get_mut(&idx)
                                        && !partial_str.is_empty() {
                                            acc.push_str(partial_str);
                                            let id = id.clone();
                                            yield Ok(StreamPart::ToolInputDelta {
                                                id,
                                                delta: partial_str.to_string(),
                                                provider_metadata: None,
                                            });
                                        }
                            if let Some(rc) = delta.get("reasoningContent") {
                                if !["text", "signature", "data", "redactedContent"].iter().any(|key| rc.get(key).and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())) { continue; }
                                let id = idx.to_string();
                                if let std::collections::hash_map::Entry::Vacant(entry) = reasoning_blocks.entry(idx) {
                                    entry.insert(None);
                                    yield Ok(StreamPart::ReasoningStart { id: id.clone(), provider_metadata: None });
                                }
                                if let Some(text) = rc.get("text").and_then(|v| v.as_str()) {
                                    yield Ok(StreamPart::ReasoningDelta { id: id.clone(), delta: text.to_string(), provider_metadata: None });
                                } else if let Some(sig) = rc.get("signature").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                    yield Ok(StreamPart::ReasoningDelta { id: id.clone(), delta: String::new(), provider_metadata: Some(options::metadata(json!({"signature": sig}))) });
                                } else if let Some(data) = rc.get("data").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                                    yield Ok(StreamPart::ReasoningDelta { id: id.clone(), delta: String::new(), provider_metadata: Some(options::metadata(json!({"redactedData": data}))) });
                                } else if let Some(data) = rc.get("redactedContent").and_then(|v| v.as_str()) {
                                    reasoning_blocks.entry(idx).or_default().get_or_insert_with(String::new).push_str(data);
                                }
                            }
                        }
                    }
                    "contentBlockStop" => {
                        let idx = payload
                            .get("contentBlockIndex")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;

                        if let Some((id, name, acc)) = tool_blocks.remove(&idx) {
                            yield Ok(StreamPart::ToolInputEnd { id: id.clone(), provider_metadata: None});
                            // Empty input normalizes to "{}" per the upstream
                            // provider.
                            let input = if acc.is_empty() {
                                "{}".to_string()
                            } else {
                                acc
                            };
                            yield Ok(StreamPart::ToolCall(RawToolCall {
                                tool_call_id: normalize_tool_call_id(&id, model_id.contains("mistral.")),
                                tool_name: name,
                                input,
                                provider_executed: None,
                                dynamic: None,
                                thought_signature: None,
                                provider_metadata: None,
                            }));
                        } else if let Some(redacted) = reasoning_blocks.remove(&idx) {
                            yield Ok(StreamPart::ReasoningEnd { id: idx.to_string(), provider_metadata: reasoning_redacted_meta(redacted) });
                        } else if text_blocks.remove(&idx) {
                            yield Ok(StreamPart::TextEnd { id: idx.to_string(), provider_metadata: None });
                        }
                    }
                    "messageStop" => {
                        if let Some(reason) =
                            payload.get("stopReason").and_then(|v| v.as_str())
                        {
                            final_finish_reason = Some(map_finish_reason(reason));
                        }
                        // #26: surface which stop sequence sentinel fired
                        // (additionalModelResponseFields.delta.stop_sequence), if any.
                        if let Some(seq) = payload
                            .get("additionalModelResponseFields")
                            .and_then(|f| f.get("delta"))
                            .and_then(|d| d.get("stop_sequence"))
                            .and_then(|v| v.as_str())
                        {
                            stop_sequence = Some(seq.to_string());
                        }
                    }
                    "metadata" => {
                        if let Some(usage) = payload.get("usage") {
                            let bedrock_usage: super::types::BedrockUsage =
                                match serde_json::from_value(usage.clone()) {
                                    Ok(usage) => usage,
                                    Err(error) => {
                                        final_finish_reason = Some(FinishReason { unified: FinishReasonUnified::Error, raw: None });
                                        yield Ok(StreamPart::Error { error: AiMuxError::InvalidResponseData(format!("{error}; usage: {usage}")) });
                                        continue;
                                    }
                                };
                            final_usage = convert_usage(Some(&bedrock_usage));
                            final_usage.raw = Some(usage.clone());
                            if let Some(cache_usage) = cache_usage_metadata(Some(&bedrock_usage)) { finish_meta.insert("usage".into(), cache_usage); }
                        }
                        // #27: surface guardrails trace, performanceConfig, and
                        // serviceTier into the Finish chunk's provider_metadata
                        // (mirrors TS `doStream` metadata handling).
                        if let Some(trace) = payload.get("trace") {
                            finish_meta.insert("trace".to_string(), trace.clone());
                        }
                        if let Some(pc) = payload.get("performanceConfig") {
                            finish_meta
                                .insert("performanceConfig".to_string(), pc.clone());
                        }
                        if let Some(st) = payload.get("serviceTier") {
                            finish_meta.insert("serviceTier".to_string(), st.clone());
                        }
                    }
                    _ => {}
                }
            }

            // #26: merge the stop sentinel into the metadata payload, then build
            // the Finish provider_metadata under `amazonBedrock` (mirrors the
            // TS `doStream` flush handler).
            if let Some(seq) = stop_sequence {
                finish_meta.insert(
                    "stopSequence".to_string(),
                    serde_json::Value::String(seq),
                );
            }
            let provider_metadata = if finish_meta.is_empty() {
                None
            } else {
                let payload = serde_json::Value::Object(finish_meta);
                Some(options::metadata(payload))
            };

            yield Ok(StreamPart::Finish {
                finish_reason: final_finish_reason.unwrap_or(FinishReason {
                    unified: FinishReasonUnified::Other,
                    raw: None,
                }),
                usage: final_usage,
                provider_metadata,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::shared::StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
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
                &tool_use.tool_use_id,
                model_id.contains("mistral."),
            ),
            tool_name: tool_use.name.clone(),
            input: tool_use.input.to_string(),
            provider_executed: None,
            dynamic: None,
            thought_signature: None,
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

fn cache_usage_metadata(usage: Option<&super::types::BedrockUsage>) -> Option<serde_json::Value> {
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
