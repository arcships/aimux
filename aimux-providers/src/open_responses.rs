//! Open Responses provider - a generic Responses API wrapper.
//!
//! Works with any OpenAI Responses-compatible API endpoint (LM Studio,
//! OpenAI, etc.). Unlike the OpenAI Chat Completions provider, this speaks
//! the Responses API wire format (`/v1/responses`).
//!
//! Translation of `reference/ai/packages/open-responses/src/responses/`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::provider::Provider;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::Tool;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ReasoningEffort, ResponseMetadata, Usage, Warning,
};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, send_stream_timed, send_timed,
};
use aimux_stream::SseStream;

// == Config ==

/// Configuration for the Open Responses provider.
///
/// Mirrors the TS `OpenResponsesConfig`. The `url` is the **full endpoint
/// URL** (e.g. `https://localhost:1234/v1/responses`), not a base URL.
pub struct OpenResponsesConfig {
    /// Provider name reported by `LanguageModel::provider`.
    pub provider: String,
    /// Key used to look up provider-specific options in `providerOptions`.
    pub provider_options_name: String,
    /// Full endpoint URL for the Responses API.
    pub url: String,
    /// Optional header factory - called on every request and merged with
    /// per-request headers (e.g. `Authorization`).
    pub headers: Option<Arc<dyn Fn() -> HashMap<String, String> + Send + Sync>>,
    /// ID generator (retained for API parity with the TS config; not used
    /// by the model itself).
    pub generate_id: Arc<dyn Fn() -> String + Send + Sync>,
    /// 凭证来源(RFC-0023):`None` = 未标注(回放时由 headers 闭包推断);
    /// `Some("explicit")` / `Some("env:VAR")` = 调用方显式标注。Open Responses
    /// 是通用包装,认证由调用方经 `headers` 闭包管理,故默认 `None`。
    pub api_key_source: Option<String>,
}

impl std::fmt::Debug for OpenResponsesConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenResponsesConfig")
            .field("provider", &self.provider)
            .field("provider_options_name", &self.provider_options_name)
            .field("url", &self.url)
            .field("headers", &self.headers.is_some())
            .field("generate_id", &"<closure>")
            .field("api_key_source", &self.api_key_source)
            .finish()
    }
}

impl OpenResponsesConfig {
    /// Create a new config with the given provider name, provider-options
    /// name, and endpoint URL. `headers` defaults to `None` and `generate_id`
    /// defaults to a simple counter.
    pub fn new(
        provider: impl Into<String>,
        provider_options_name: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            provider_options_name: provider_options_name.into(),
            url: url.into(),
            headers: None,
            generate_id: Arc::new(|| {
                use std::sync::atomic::{AtomicU64, Ordering};
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                let n = COUNTER.fetch_add(1, Ordering::Relaxed);
                format!("id-{n}")
            }),
            api_key_source: None,
        }
    }

    /// Set a header factory.
    #[must_use]
    pub fn with_headers<F>(mut self, headers: F) -> Self
    where
        F: Fn() -> HashMap<String, String> + Send + Sync + 'static,
    {
        self.headers = Some(Arc::new(headers));
        self
    }

    /// Set a generate-id factory.
    #[must_use]
    pub fn with_generate_id<F>(mut self, generate_id: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        self.generate_id = Arc::new(generate_id);
        self
    }

    /// 标注凭证来源(RFC-0023 回放重建用)。Open Responses 的认证由调用方经
    /// `headers` 闭包管理;此 setter 让调用方显式标注来源(如 `env:VAR`),
    /// 覆盖 `config_snapshot` 对闭包的推断。
    #[must_use]
    pub fn with_api_key_source(mut self, source: Option<&str>) -> Self {
        self.api_key_source = source.map(std::string::ToString::to_string);
        self
    }
}

// == Provider ==

/// Open Responses provider - creates [`OpenResponsesModel`] instances.
pub struct OpenResponsesProvider {
    config: OpenResponsesConfig,
}

impl OpenResponsesProvider {
    #[must_use]
    pub fn new(config: OpenResponsesConfig) -> Self {
        Self { config }
    }

    /// Create a model instance for the given model id.
    #[must_use]
    pub fn model(&self, model_id: &str) -> OpenResponsesModel {
        OpenResponsesModel::new(model_id.to_string(), &self.config)
    }
}

impl Provider for OpenResponsesProvider {
    fn name(&self) -> &str {
        &self.config.provider
    }

    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Ok(Box::new(self.model(model_id)))
    }
}

// == Model ==

/// An Open Responses language model.
pub struct OpenResponsesModel {
    model_id: String,
    config: OpenResponsesConfig,
}

impl OpenResponsesModel {
    #[must_use]
    pub fn new(model_id: String, config: &OpenResponsesConfig) -> Self {
        Self {
            model_id,
            config: OpenResponsesConfig {
                provider: config.provider.clone(),
                provider_options_name: config.provider_options_name.clone(),
                url: config.url.clone(),
                headers: config.headers.clone(),
                generate_id: config.generate_id.clone(),
                api_key_source: config.api_key_source.clone(),
            },
        }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = self
            .config
            .headers
            .as_ref()
            .map(|h| h())
            .unwrap_or_default();
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    /// Resolve the credential source for `config_snapshot` (M2b).
    ///
    /// Priority:
    /// 1. An explicit `api_key_source` set via [`OpenResponsesConfig::with_api_key_source`]
    ///    (e.g. `"env:VAR"`) — lets callers mark env-sourced auth precisely.
    /// 2. Otherwise, inspect the `headers` closure for a known auth header
    ///    *key* (`authorization` / `x-api-key` / `x-goog-api-key` / `api-key`).
    ///    Only key names are inspected — header *values* are never recorded, so
    ///    no secret leaks. If an auth header is present the source is
    ///    `"explicit"` (caller-managed); this is the same closure already
    ///    invoked by `build_headers` on every request, so calling it here is
    ///    no riskier.
    /// 3. No headers closure (and no explicit source) → `"none"` (e.g. a local
    ///    LM Studio server with no auth).
    fn resolve_api_key_source(&self) -> String {
        if let Some(source) = &self.config.api_key_source {
            return source.clone();
        }
        let Some(headers_factory) = &self.config.headers else {
            return "none".to_string();
        };
        let headers = headers_factory();
        let has_auth = headers.keys().any(|k| {
            let lower = k.to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "authorization" | "x-api-key" | "x-goog-api-key" | "api-key"
            )
        });
        if has_auth {
            "explicit".to_string()
        } else {
            "none".to_string()
        }
    }
}

#[async_trait]
impl LanguageModel for OpenResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn config_snapshot(&self) -> aimux_core::recording::ProviderRecord {
        use aimux_core::recording::ProviderRecord;
        // M2b: generic Responses wrapper — auth is caller-managed via the
        // `headers` closure. Record the real credential source: an explicit
        // marker if set, else inferred from the closure (auth header key
        // present → "explicit"; no closure → "none"). Only key names are
        // inspected — header values (secrets) are never serialized.
        ProviderRecord {
            provider: self.provider().to_string(),
            model_id: self.model_id.clone(),
            base_url: Some(self.config.url.clone()),
            api_key_source: self.resolve_api_key_source(),
            profile: None,
            provider_options: Some(serde_json::json!({
                "provider_options_name": self.config.provider_options_name,
                "has_headers": self.config.headers.is_some(),
            })),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let (body, warnings) =
            build_request_body(&self.model_id, options, &self.config.provider_options_name);

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.config.url.clone(),
                headers: headers.into_iter().collect(),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let raw: Value = serde_json::from_slice(&resp.body)?;

        // Check for response.error first (surfaces before the no-output fallback).
        if let Some(error) = raw.get("error").and_then(|e| e.as_object())
            && !error.is_empty()
        {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(AiMuxError::ApiCall(ApiCallError {
                status_code: Some(resp.status),
                provider_code: error
                    .get("code")
                    .or_else(|| error.get("type"))
                    .and_then(|c| c.as_str())
                    .map(std::string::ToString::to_string),
                message: message.to_string(),
                response_body: Some(String::from_utf8_lossy(&resp.body).into_owned()),
                ..Default::default()
            }));
        }

        // Check for null/missing output.
        let output = raw.get("output");
        if output.map(serde_json::Value::is_null).unwrap_or(true) {
            let detail = raw
                .get("incomplete_details")
                .and_then(|d| d.get("reason"))
                .and_then(|r| r.as_str())
                .or_else(|| raw.get("status").and_then(|s| s.as_str()));
            let message = match detail {
                Some(d) => format!("Responses API returned no output ({d})"),
                None => "Responses API returned no output".to_string(),
            };
            return Err(AiMuxError::InvalidResponseData(message));
        }

        // Build content array from output items.
        let mut content = Vec::new();
        let mut has_tool_calls = false;

        if let Some(output_arr) = output.and_then(|o| o.as_array()) {
            for part in output_arr {
                let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match part_type {
                    "reasoning" => {
                        if let Some(content_parts) = part.get("content").and_then(|c| c.as_array())
                        {
                            for cp in content_parts {
                                if let Some(text) = cp.get("text").and_then(|t| t.as_str()) {
                                    content.push(GenerateContent::Reasoning {
                                        text: text.to_string(),
                                        provider_metadata: None,
                                    });
                                }
                            }
                        }
                    }
                    "message" => {
                        if let Some(content_parts) = part.get("content").and_then(|c| c.as_array())
                        {
                            for cp in content_parts {
                                if let Some(text) = cp.get("text").and_then(|t| t.as_str()) {
                                    content.push(GenerateContent::Text {
                                        text: text.to_string(),
                                        provider_metadata: None,
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
                        content.push(GenerateContent::ToolCall {
                            tool_call_id: call_id,
                            tool_name: name,
                            input,
                            provider_executed: None,
                            dynamic: None,
                            thought_signature: None,
                            provider_metadata: None,
                        });
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

        let response = ResponseMetadata {
            id: raw
                .get("id")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            timestamp: raw
                .get("created_at")
                .and_then(serde_json::Value::as_u64)
                .map(|ts| format!("{ts}")),
            model_id: raw
                .get("model")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
        };

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings,
            provider_metadata: None,
            response,
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let (body, warnings) =
            build_request_body(&self.model_id, options, &self.config.provider_options_name);

        let stream_body = {
            let mut b = body.clone();
            if let Some(obj) = b.as_object_mut() {
                obj.insert("stream".to_string(), json!(true));
            }
            b
        };

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.config.url.clone(),
                headers: headers.into_iter().collect(),
                body: HttpBody::Json(stream_body),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
                recording_context: options.recording_context.clone(),
            },
            RetryConfig::default(),
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let mut sse_stream = SseStream::new(resp.body);

        // Peek at the first SSE event to detect early errors.
        let first_event = sse_stream.next().await;
        if let Some(Ok(ref event)) = first_event
            && let Ok(val) = serde_json::from_str::<Value>(&event.data)
            && let Some(err_obj) = val.get("error")
        {
            let message = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("stream error");
            return Err(AiMuxError::ApiCall(ApiCallError {
                status_code: Some(resp.status),
                provider_code: err_obj
                    .get("code")
                    .or_else(|| err_obj.get("type"))
                    .and_then(|c| c.as_str())
                    .map(std::string::ToString::to_string),
                message: message.to_string(),
                response_body: Some(event.data.clone()),
                ..Default::default()
            }));
        }

        let stream = async_stream::stream! {
            // First part: StreamStart.
            yield Ok(StreamPart::StreamStart { warnings });

            let mut final_usage = Usage::default();
            let mut has_tool_calls = false;
            let mut finish_reason = FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            };
            let mut is_active_reasoning = false;

            // Tool-call accumulators keyed by item_id.
            let mut tool_calls: HashMap<String, ToolCallAccum> = HashMap::new();

            let mut event_iter =
                futures::stream::iter(first_event.into_iter()).chain(sse_stream);

            while let Some(event) = event_iter.next().await {
                match event {
                    Ok(sse_event) => {
                        if sse_event.data == "[DONE]" {
                            break;
                        }

                        let chunk: Value = match serde_json::from_str(&sse_event.data) {
                            Ok(v) => v,
                            Err(e) => {
                                yield Ok(StreamPart::Error { error: e.into() });
                                break;
                            }
                        };

                        let chunk_type = chunk
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");

                        match chunk_type {
                            // -- Tool call / reasoning / message item added --
                            "response.output_item.added" => {
                                if let Some(item) = chunk.get("item") {
                                    let item_type = item
                                        .get("type")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("");
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
                                            yield Ok(StreamPart::ReasoningStart {
                                                id,
                                                provider_metadata: None,
                                            });
                                            is_active_reasoning = true;
                                        }
                                        "message" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            yield Ok(StreamPart::TextStart { id, provider_metadata: None});
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
                                    let item_type = item
                                        .get("type")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("");
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
                                            let arguments = accum
                                                .as_ref()
                                                .and_then(|a| a.arguments.clone())
                                                .or_else(|| {
                                                    item.get("arguments")
                                                        .and_then(|v| v.as_str())
                                                        .map(std::string::ToString::to_string)
                                                })
                                                .unwrap_or_default();
                                            let input = Value::String(arguments);
                                            yield Ok(StreamPart::ToolCall {
                                                tool_call_id,
                                                tool_name,
                                                input,
                                                provider_executed: None,
                                                dynamic: None,
                                                thought_signature: None,
                                                invalid: None,
                                                error: None,
                                                provider_metadata: None,
                                            });
                                            has_tool_calls = true;
                                        }
                                        "reasoning" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            yield Ok(StreamPart::ReasoningEnd {
                                                id,
                                                provider_metadata: None,
                                            });
                                            is_active_reasoning = false;
                                        }
                                        "message" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            yield Ok(StreamPart::TextEnd { id, provider_metadata: None});
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            // -- Reasoning text delta (LM Studio extension) --
                            "response.reasoning_text.delta" => {
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
                                yield Ok(StreamPart::ReasoningDelta {
                                    id,
                                    delta,
                                    provider_metadata: None,
                                });
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
                                yield Ok(StreamPart::TextDelta { id, delta, provider_metadata: None});
                            }

                            // -- Completion events --
                            "response.completed" | "response.incomplete" => {
                                if let Some(response) = chunk.get("response") {
                                    let reason = response
                                        .get("incomplete_details")
                                        .and_then(|d| d.get("reason"))
                                        .and_then(|r| r.as_str());
                                    finish_reason = FinishReason {
                                        unified: map_open_responses_finish_reason(
                                            reason,
                                            has_tool_calls,
                                        ),
                                        raw: reason.map(std::string::ToString::to_string),
                                    };
                                    if let Some(usage_val) = response.get("usage") {
                                        final_usage = extract_usage_from_value(usage_val);
                                    }
                                }
                            }
                            "response.failed" => {
                                if let Some(response) = chunk.get("response") {
                                    let raw = response
                                        .get("error")
                                        .and_then(|e| e.get("code"))
                                        .and_then(|c| c.as_str())
                                        .or_else(|| {
                                            response.get("status").and_then(|s| s.as_str())
                                        });
                                    finish_reason = FinishReason {
                                        unified: FinishReasonUnified::Error,
                                        raw: raw.map(std::string::ToString::to_string),
                                    };
                                    if let Some(usage_val) = response.get("usage") {
                                        final_usage = extract_usage_from_value(usage_val);
                                    }
                                }
                            }
                            _ => {
                                // Ignore unrecognised event types.
                            }
                        }
                    }
                    Err(e) => {
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::InvalidResponseData(e.to_string()),
                        });
                        break;
                    }
                }
            }

            // Flush: close any dangling reasoning segment.
            if is_active_reasoning {
                yield Ok(StreamPart::ReasoningEnd {
                    id: "reasoning-0".to_string(),
                    provider_metadata: None,
                });
            }

            yield Ok(StreamPart::Finish {
                finish_reason,
                usage: final_usage,
                provider_metadata: None,
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

// == Tool call accumulator ==

struct ToolCallAccum {
    tool_name: Option<String>,
    tool_call_id: Option<String>,
    arguments: Option<String>,
}

// == Finish reason mapping ==

/// Map an Open Responses finish reason to the unified enum.
///
/// Mirrors the TS `mapOpenResponsesFinishReason`.
#[must_use]
pub fn map_open_responses_finish_reason(
    finish_reason: Option<&str>,
    has_tool_calls: bool,
) -> FinishReasonUnified {
    match finish_reason {
        None => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        Some("max_output_tokens") => FinishReasonUnified::Length,
        Some("content_filter") => FinishReasonUnified::ContentFilter,
        Some(_) => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Other
            }
        }
    }
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
    let (input, instructions, input_warnings) = convert_to_open_responses_input(&options.prompt);
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
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    // Convert tool choice.
    // Only emit when not the default (Auto) — Rust's ToolChoice::Auto is the
    // default and indistinguishable from "not set", matching the TS behavior
    // where undefined toolChoice is omitted from the body.
    let converted_tool_choice: Option<Value> = match &options.tool_choice {
        ToolChoice::Auto => None,
        ToolChoice::None => Some(json!("none")),
        ToolChoice::Required => Some(json!("required")),
        ToolChoice::Tool { tool_name } => Some(json!({
            "type": "function",
            "name": tool_name,
        })),
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
                Some(json!({ "type": "json_schema" }))
            }
        }
        _ => None,
    };

    // Resolve reasoning effort from top-level reasoning option.
    let resolved_reasoning_effort: Option<String> =
        if options.reasoning.is_some_and(ReasoningEffort::is_custom) {
            match options.reasoning.unwrap() {
                ReasoningEffort::None => Some("none".to_string()),
                ReasoningEffort::Minimal => Some("low".to_string()),
                ReasoningEffort::Low => Some("low".to_string()),
                ReasoningEffort::Medium => Some("medium".to_string()),
                ReasoningEffort::High => Some("high".to_string()),
                ReasoningEffort::Xhigh => Some("xhigh".to_string()),
                ReasoningEffort::ProviderDefault => None,
            }
        } else {
            None
        };

    // Resolve reasoning summary from provider options.
    let reasoning_summary: Option<String> = options
        .provider_options
        .as_ref()
        .and_then(|m| m.get(provider_options_name))
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

// == Prompt conversion ==

/// Convert a `LanguageModelPrompt` to the Open Responses input format.
///
/// Mirrors the TS `convertToOpenResponsesInput`. System messages become
/// `instructions`; user/assistant/tool messages become `input` items.
#[must_use]
pub fn convert_to_open_responses_input(
    prompt: &LanguageModelPrompt,
) -> (Value, Option<String>, Vec<Warning>) {
    let mut input: Vec<Value> = Vec::new();
    let mut warnings = Vec::new();
    let mut system_messages: Vec<String> = Vec::new();

    for msg in prompt {
        match msg.role {
            Role::System => {
                for part in &msg.content {
                    if let ContentPart::Text { text, .. } = part {
                        system_messages.push(text.clone());
                    }
                }
            }
            Role::User => {
                let user_content = convert_user_content(&msg.content, &mut warnings);
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": user_content,
                }));
            }
            Role::Assistant => {
                let mut assistant_content: Vec<Value> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();

                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } => {
                            assistant_content.push(json!({
                                "type": "output_text",
                                "text": text,
                            }));
                        }
                        ContentPart::ToolCall {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            ..
                        } => {
                            let arguments = match tool_input {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            tool_calls.push(json!({
                                "type": "function_call",
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": arguments,
                            }));
                        }
                        _ => {}
                    }
                }

                if !assistant_content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": assistant_content,
                    }));
                }
                for tc in tool_calls {
                    input.push(tc);
                }
            }
            Role::Tool => {
                for part in &msg.content {
                    if let ContentPart::ToolResult {
                        tool_call_id,
                        result,
                        ..
                    } = part
                    {
                        let content_value = resolve_tool_result_output(result, &mut warnings);
                        input.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_call_id,
                            "output": content_value,
                        }));
                    }
                }
            }
        }
    }

    let instructions = if system_messages.is_empty() {
        None
    } else {
        Some(system_messages.join("\n"))
    };

    (json!(input), instructions, warnings)
}

/// Convert user message content parts to the Open Responses format.
fn convert_user_content(content: &[ContentPart], warnings: &mut Vec<Warning>) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    for part in content {
        match part {
            ContentPart::Text { text, .. } => {
                parts.push(json!({ "type": "input_text", "text": text }));
            }
            ContentPart::Image {
                image, media_type, ..
            } => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(image);
                parts.push(json!({
                    "type": "input_image",
                    "image_url": format!("data:{};base64,{}", media_type, b64),
                }));
            }
            ContentPart::File {
                data,
                media_type,
                filename,
                ..
            } => {
                let top_level = top_level_media_type(media_type);
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                if top_level == "image" {
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", media_type, b64),
                    }));
                } else {
                    parts.push(json!({
                        "type": "input_file",
                        "filename": filename.as_deref().unwrap_or("data"),
                        "file_data": format!("data:{};base64,{}", media_type, b64),
                    }));
                }
            }
            ContentPart::FileBase64 {
                data,
                media_type,
                filename,
                ..
            } => {
                let top_level = top_level_media_type(media_type);
                if top_level == "image" {
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", media_type, data),
                    }));
                } else {
                    parts.push(json!({
                        "type": "input_file",
                        "filename": filename.as_deref().unwrap_or("data"),
                        "file_data": format!("data:{};base64,{}", media_type, data),
                    }));
                }
            }
            ContentPart::FileUrl {
                url, media_type, ..
            } => {
                let top_level = top_level_media_type(media_type);
                if top_level == "image" {
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": url,
                    }));
                } else {
                    parts.push(json!({
                        "type": "input_file",
                        "file_url": url,
                    }));
                }
            }
            ContentPart::FileReference { .. } => {
                warnings.push(Warning::Other {
                    message: "unsupported file part with provider reference".to_string(),
                });
            }
            _ => {
                warnings.push(Warning::Other {
                    message: format!("unsupported content part type: {}", part_variant_name(part)),
                });
            }
        }
    }
    json!(parts)
}

/// Resolve a tool-result `output` value into the Open Responses `output`
/// field, mirroring the TS convert logic.
fn resolve_tool_result_output(output: &Value, warnings: &mut Vec<Warning>) -> Value {
    let output_type = output.get("type").and_then(|x| x.as_str());

    match output_type {
        Some("text") | Some("error-text") => {
            output.get("value").cloned().unwrap_or_else(|| Value::Null)
        }
        Some("execution-denied") => output
            .get("reason")
            .and_then(|r| r.as_str())
            .map(|s| Value::String(s.to_string()))
            .unwrap_or_else(|| Value::String("Tool call execution denied.".to_string())),
        Some("json") | Some("error-json") => {
            let v = output.get("value").unwrap_or(&Value::Null);
            Value::String(v.to_string())
        }
        Some("content") => {
            let v = output.get("value");
            if let Some(arr) = v.and_then(|v| v.as_array()) {
                let mut parts: Vec<Value> = Vec::new();
                for item in arr {
                    match item.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                parts.push(json!({ "type": "input_text", "text": text }));
                            }
                        }
                        Some("file") => {
                            if let Some(media_type) = item.get("mediaType").and_then(|m| m.as_str())
                            {
                                let top_level = top_level_media_type(media_type);
                                if let Some(data) = item.get("data") {
                                    if let Some(data_obj) =
                                        data.get("data").and_then(|d| d.as_str())
                                    {
                                        if top_level == "image" {
                                            parts.push(json!({
                                                "type": "input_image",
                                                "image_url": format!("data:{};base64,{}", media_type, data_obj),
                                            }));
                                        } else {
                                            parts.push(json!({
                                                "type": "input_file",
                                                "filename": item.get("filename").and_then(|f| f.as_str()).unwrap_or("data"),
                                                "file_data": format!("data:{};base64,{}", media_type, data_obj),
                                            }));
                                        }
                                    } else if let Some(url) =
                                        data.get("url").and_then(|u| u.as_str())
                                    {
                                        if top_level == "image" {
                                            parts.push(json!({
                                                "type": "input_image",
                                                "image_url": url,
                                            }));
                                        } else {
                                            parts.push(json!({
                                                "type": "input_file",
                                                "file_url": url,
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {
                            warnings.push(Warning::Other {
                                message: format!(
                                    "unsupported tool content part type: {}",
                                    item.get("type")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("unknown")
                                ),
                            });
                        }
                    }
                }
                Value::Array(parts)
            } else {
                output.clone()
            }
        }
        _ => output.clone(),
    }
}

/// Extract the top-level media type (e.g. "image" from "image/png").
fn top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}

/// Get a human-readable name for a ContentPart variant (for warnings).
fn part_variant_name(part: &ContentPart) -> &'static str {
    match part {
        ContentPart::Text { .. } => "text",
        ContentPart::Image { .. } => "image",
        ContentPart::File { .. } => "file",
        ContentPart::FileBase64 { .. } => "file-base64",
        ContentPart::FileUrl { .. } => "file-url",
        ContentPart::FileReference { .. } => "file-reference",
        ContentPart::Reasoning { .. } => "reasoning",
        ContentPart::ToolCall { .. } => "tool-call",
        ContentPart::ToolResult { .. } => "tool-result",
    }
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
        input_tokens: aimux_core::types::TokenUsage {
            total: input_tokens,
            no_cache: Some(input_tokens.unwrap_or(0) - cached_input_tokens.unwrap_or(0)),
            cache_read: cached_input_tokens,
            cache_write: None,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: output_tokens,
            text: Some(output_tokens.unwrap_or(0) - reasoning_tokens.unwrap_or(0)),
            reasoning: reasoning_tokens,
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: Some(usage.clone()),
    }
}
