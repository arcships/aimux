//! Amazon Bedrock language model — implements `LanguageModel`.
//!
//! Uses the Bedrock Converse API (`/model/{model-id}/converse` and
//! `/model/{model-id}/converse-stream`), which provides a unified interface
//! across all Bedrock-backed models (Anthropic Claude, Meta Llama, Mistral,
//! etc.).

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use serde_json::json;

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, send_stream_timed, send_timed,
};

use super::BedrockAuth;
use super::convert::{build_request_body, convert_usage, map_finish_reason};
use super::sigv4::sign_request;
use super::types::{BedrockContentBlock, BedrockConverseResponse};

/// An Amazon Bedrock language model (e.g. `anthropic.claude-3-5-sonnet-20240620-v1:0`).
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct BedrockModel {
    model_id: String,
    config: BedrockConfig,
}

/// Configuration for a Bedrock model instance (cloned from the provider).
#[derive(Debug, Clone)]
pub struct BedrockConfig {
    pub base_url: String,
    pub auth: BedrockAuth,
    /// 重试配置(M1b)。默认 `RetryConfig::default()`。
    pub retry_config: RetryConfig,
    /// 凭证来源(RFC-0023):`None` = explicit;`Some("env:VAR")` = 环境变量。
    /// SigV4 记 access-key 来源;BearerToken 记 bearer-token 来源。不存明文。
    pub api_key_source: Option<String>,
}

impl BedrockModel {
    #[must_use]
    pub fn new(model_id: String, config: BedrockConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self, stream: bool) -> String {
        let suffix = if stream {
            "converse-stream"
        } else {
            "converse"
        };
        // Bedrock model IDs contain dots and colons (e.g.
        // `anthropic.claude-3-5-sonnet-20240620-v1:0`). These characters are
        // valid in URL paths and are sent unencoded — matching the AWS CLI
        // and SDK behaviour.
        format!(
            "{}/model/{}/{}",
            self.config.base_url, self.model_id, suffix
        )
    }

    fn build_headers(
        &self,
        body: &str,
        url: &str,
        extra: Option<&HashMap<String, String>>,
    ) -> Result<Vec<(String, String)>, AiMuxError> {
        let mut extra_headers: Vec<(String, String)> = Vec::new();
        if let Some(extra) = extra {
            for (k, v) in extra {
                extra_headers.push((k.clone(), v.clone()));
            }
        }

        match &self.config.auth {
            BedrockAuth::BearerToken(token) => {
                let mut headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
                headers.extend(extra_headers);
                Ok(headers)
            }
            BedrockAuth::SigV4(creds) => {
                let signed = sign_request(creds, "bedrock", "POST", url, body, &extra_headers);
                let mut headers: Vec<(String, String)> = Vec::new();
                for (k, v) in &signed.headers {
                    headers.push((k.clone(), v.clone()));
                }
                Ok(headers)
            }
        }
    }
}

#[async_trait]
impl LanguageModel for BedrockModel {
    fn provider(&self) -> &str {
        "amazon-bedrock"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        // M2b: record identity + credential source + region (encoded in
        // base_url) + retry. Never serialize the SigV4 credentials or bearer
        // token plaintext — only the auth *kind* and source marker.
        let auth_kind = match &self.config.auth {
            BedrockAuth::BearerToken(_) => "bearer_token",
            BedrockAuth::SigV4(_) => "sigv4",
        };
        ProviderRecord {
            provider: self.provider().to_string(),
            model_id: self.model_id.clone(),
            base_url: Some(self.config.base_url.clone()),
            api_key_source: self
                .config
                .api_key_source
                .clone()
                .unwrap_or_else(|| "explicit".to_string()),
            profile: None,
            provider_options: Some(serde_json::json!({
                "auth_kind": auth_kind,
                "max_retries": self.config.retry_config.max_retries,
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = self.endpoint(false);
        let headers = self.build_headers(&body_str, &url, options.headers.as_ref())?;
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let data: BedrockConverseResponse =
            serde_json::from_slice(&resp.body).map_err(AiMuxError::from)?;

        // Extract content from response.output.message.content
        let mut content = Vec::new();
        if let Some(output) = &data.output
            && let Some(message) = &output.message
        {
            for block in &message.content {
                extract_content(block, &mut content);
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

        let usage = convert_usage(data.usage.as_ref());

        let request_id = response_headers
            .get("x-amzn-requestid")
            .cloned()
            .or_else(|| response_headers.get("x-amzn-request-id").cloned());

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: Vec::new(),
            provider_metadata: None,
            response: ResponseMetadata {
                id: request_id,
                timestamp: response_headers.get("date").cloned(),
                model_id: Some(self.model_id.clone()),
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let body = build_request_body(&self.model_id, options);
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        let url = self.endpoint(true);
        let headers = self.build_headers(&body_str, &url, options.headers.as_ref())?;
        let retry_config = crate::openai::model::resolve_retry_config(
            &self.config.retry_config,
            options.max_retries,
        );

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url,
                headers,
                body: HttpBody::Bytes(body_str.into_bytes(), "application/json".to_string()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        // Bedrock converse-stream returns binary AWS event stream format.
        // We read the full body and decode it, then emit stream parts.
        // (For true streaming we'd decode incrementally, but the Bedrock event
        // stream codec requires buffering whole frames anyway.)
        let mut buf: Vec<u8> = Vec::new();
        let mut body_stream = resp.body;
        while let Some(chunk) = body_stream.next().await {
            match chunk {
                Ok(bytes) => buf.extend_from_slice(&bytes),
                Err(e) => return Err(e),
            }
        }
        let response_bytes = buf;

        let request_id = response_headers
            .get("x-amzn-requestid")
            .cloned()
            .or_else(|| response_headers.get("x-amzn-request-id").cloned());
        // Same source as the non-stream path: the response `Date` header
        // (RFC1123). Keeps stream/non-stream timestamps consistent.
        let response_timestamp = response_headers.get("date").cloned();

        let model_id = self.model_id.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings: vec![] });

            yield Ok(StreamPart::ResponseMetadata {
                id: request_id,
                timestamp: response_timestamp,
                model_id: Some(model_id.clone()),
            });

            let messages = super::event_stream::decode_messages(&response_bytes);

            let mut text_id: Option<String> = None;
            let mut reasoning_id: Option<String> = None;
            // Reasoning signatures arrive in their own reasoningContent delta
            // (usually with no text); accumulated here and attached to the
            // concluding ReasoningEnd.
            let mut reasoning_signature: Option<String> = None;
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

            for msg in &messages {
                if msg.message_type != "event" {
                    continue;
                }

                let payload: serde_json::Value = match serde_json::from_str(&msg.data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

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
                                let id = if id.is_empty() {
                                    format!("call-{idx}")
                                } else {
                                    id
                                };
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
                                text_id = Some(id.clone());
                                yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                            }
                        } else {
                            // Default: text block.
                            let id = idx.to_string();
                            text_id = Some(id.clone());
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
                                    if text_id.is_none() {
                                        let id = idx.to_string();
                                        text_id = Some(id.clone());
                                        yield Ok(StreamPart::TextStart { id, provider_metadata: None});
                                    }
                                    if let Some(id) = &text_id {
                                        yield Ok(StreamPart::TextDelta {
                                            id: id.clone(),
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
                            // Reasoning delta — `reasoningContent.text` carries
                            // incremental reasoning text; `reasoningContent.signature`
                            // carries the reasoning signature (typically in a
                            // final, text-less delta). The signature is attached
                            // to the concluding ReasoningEnd via provider_metadata
                            // so extended-thinking multi-turn can echo it back —
                            // same shape as the non-streaming path. Empty text
                            // deltas are skipped.
                            if let Some(rc) = delta.get("reasoningContent") {
                                if let Some(text) = rc.get("text").and_then(|v| v.as_str())
                                    && !text.is_empty()
                                {
                                    let id = idx.to_string();
                                    if reasoning_id.as_deref() != Some(id.as_str()) {
                                        reasoning_id = Some(id.clone());
                                        yield Ok(StreamPart::ReasoningStart {
                                            id,
                                            provider_metadata: None,
                                        });
                                    }
                                    yield Ok(StreamPart::ReasoningDelta {
                                        id: idx.to_string(),
                                        delta: text.to_string(),
                                        provider_metadata: None,
                                    });
                                }
                                if let Some(sig) =
                                    rc.get("signature").and_then(|v| v.as_str())
                                {
                                    reasoning_signature
                                        .get_or_insert_with(String::new)
                                        .push_str(sig);
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
                            let input = serde_json::Value::String(if acc.is_empty() {
                                "{}".to_string()
                            } else {
                                acc
                            });
                            yield Ok(StreamPart::ToolCall {
                                tool_call_id: id,
                                tool_name: name,
                                input,
                                provider_executed: None,
                                dynamic: None,
                                thought_signature: None,
                                invalid: None,
                                error: None,
                                provider_metadata: None,
                            });
                        } else if reasoning_id.is_some() {
                            let id = idx.to_string();
                            if reasoning_id.as_deref() == Some(id.as_str()) {
                                let provider_metadata =
                                    reasoning_signature_meta(reasoning_signature.take());
                                yield Ok(StreamPart::ReasoningEnd {
                                    id,
                                    provider_metadata,
                                });
                                reasoning_id = None;
                            }
                        } else if text_id.is_some() {
                            // Only end if this is the current text block.
                            let id = idx.to_string();
                            if text_id.as_deref() == Some(id.as_str()) {
                                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
                                text_id = None;
                            }
                        }
                    }
                    "messageStop" => {
                        if let Some(reason) =
                            payload.get("stopReason").and_then(|v| v.as_str())
                        {
                            if let Some(id) = text_id.take() {
                                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
                            }
                            if let Some(id) = reasoning_id.take() {
                                let provider_metadata =
                                    reasoning_signature_meta(reasoning_signature.take());
                                yield Ok(StreamPart::ReasoningEnd {
                                    id,
                                    provider_metadata,
                                });
                            }
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
                                serde_json::from_value(usage.clone()).unwrap_or_default();
                            final_usage = convert_usage(Some(&bedrock_usage));
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

            // Close any remaining text block.
            if let Some(id) = text_id.take() {
                yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
            }
            // Close any remaining reasoning block (truncated stream without
            // contentBlockStop): still attach the accumulated signature.
            if let Some(id) = reasoning_id.take() {
                let provider_metadata =
                    reasoning_signature_meta(reasoning_signature.take());
                yield Ok(StreamPart::ReasoningEnd { id,
                provider_metadata,
            });
            }

            // #26: merge the stop sentinel into the metadata payload, then build
            // the Finish provider_metadata under both `amazonBedrock` and
            // `bedrock` keys (mirrors the TS `doStream` flush handler).
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
                Some(json!({
                    "amazonBedrock": payload,
                    "bedrock": payload,
                }))
            };

            yield Ok(StreamPart::Finish {
                finish_reason: final_finish_reason.unwrap_or(FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                }),
                usage: final_usage,
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

/// Wrap the accumulated reasoning signature as provider_metadata in the same
/// dual-key shape the non-streaming path emits (`amazonBedrock` + `bedrock`),
/// so consumers reading either key see it.
fn reasoning_signature_meta(sig: Option<String>) -> Option<serde_json::Value> {
    sig.map(|s| {
        json!({
            "amazonBedrock": { "signature": &s },
            "bedrock": { "signature": s }
        })
    })
}

/// Extract `GenerateContent` items from a non-streaming content block.
///
/// Bedrock content blocks are field-tagged: each block carries exactly one of
/// `text`, `toolUse`, or `reasoningContent`. Empty `text` blocks are preserved
/// (matching the TS SDK) so that empty text between reasoning blocks survives.
/// `reasoningContent.reasoningText` yields a `Reasoning` item whose
/// `provider_metadata` carries the `signature` under both `amazonBedrock` and
/// `bedrock` keys (or `None` when no signature is present).
/// `reasoningContent.redactedReasoning` yields a `Reasoning` item with empty
/// text and `redactedData` under both metadata keys.
fn extract_content(block: &BedrockContentBlock, content: &mut Vec<GenerateContent>) {
    if let Some(text) = &block.text {
        content.push(GenerateContent::Text {
            text: text.clone(),
            provider_metadata: None,
        });
    }
    if let Some(tool_use) = &block.tool_use {
        content.push(GenerateContent::ToolCall {
            tool_call_id: tool_use.tool_use_id.clone(),
            tool_name: tool_use.name.clone(),
            input: serde_json::Value::String(tool_use.input.to_string()),
            provider_executed: None,
            dynamic: None,
            thought_signature: None,
            provider_metadata: None,
        });
    }
    if let Some(rc) = &block.reasoning_content {
        if let Some(rt) = rc.get("reasoningText") {
            let text = rt
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let provider_metadata = rt.get("signature").and_then(|v| v.as_str()).map(|sig| {
                json!({
                    "amazonBedrock": { "signature": sig },
                    "bedrock": { "signature": sig }
                })
            });
            content.push(GenerateContent::Reasoning {
                text,
                provider_metadata,
            });
        } else if let Some(rr) = rc.get("redactedReasoning") {
            let data = rr
                .get("data")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let provider_metadata = Some(json!({
                "amazonBedrock": { "redactedData": data },
                "bedrock": { "redactedData": data }
            }));
            content.push(GenerateContent::Reasoning {
                text: String::new(),
                provider_metadata,
            });
        }
    }
}

// Suppress unused import warning for SseStream (kept for potential future use
// with SSE-based Bedrock variants).
#[allow(unused)]
use super::event_stream;
