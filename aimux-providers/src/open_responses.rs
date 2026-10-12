//! Open Responses provider - a generic Responses API wrapper.
//!
//! Works with any OpenAI Responses-compatible API endpoint (LM Studio,
//! OpenAI, etc.). Unlike the OpenAI Chat Completions provider, this speaks
//! the Responses API wire format (`/v1/responses`).
//!
//! Translation of `reference/ai/packages/open-responses/src/responses/`.

use aimux_core::tool::RawToolCall;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, TextPart, ToolCallPart,
    ToolPart, ToolResultContent, ToolResultOutput, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, StreamResult};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::Tool;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ReasoningEffort, ResponseMetadata, Usage, Warning,
};

use aimux_core::embedding_model::EmbeddingModel;
use aimux_core::image_model::ImageModel;
use aimux_core::provider::Provider;
use aimux_provider_utils::{
    FetchFunction, HeaderMapOpt, HeadersFn, Resolvable, combine_headers, validate_base_url,
};

use crate::shared::{Credential, EndpointConfig, TransformRequestBody, provider_headers};

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
        let status = input.response.status().as_u16();
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
            return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                status_code: Some(status),
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
        Ok(output)
    })
}

// == Settings ==

/// Settings of [`create_open_responses`].
///
/// Everything but `api_key` and `headers` is fixed when the provider is
/// created; those two are evaluated on every request.
#[derive(Clone)]
pub struct OpenResponsesProviderSettings {
    /// The provider name: `provider()` is `"{name}.responses"` and its first
    /// dot-separated segment is the providerOptions key the model reads.
    pub name: String,
    /// Base URL of the server (e.g. `http://localhost:1234/v1`); requests go
    /// to `{base_url}/responses`. A trailing slash is removed.
    pub base_url: String,
    /// The API key, sent as `Authorization: Bearer`. `None` sends no
    /// credential (a local server) and reads no environment variable. An
    /// explicit value is used as given, `""` included.
    pub api_key: Option<Resolvable<String>>,
    /// Extra headers, resolved on every request (a `None` value removes a
    /// header, including `Authorization`). Per-call headers win over these.
    pub headers: Option<Resolvable<HeaderMapOpt>>,
    /// The transport: a mock, a signing decorator, a proxy-aware client.
    /// `None` uses the process default, resolved per request.
    pub fetch: Option<FetchFunction>,
    /// Rewrites every JSON request body once, after it is serialized and
    /// before it is sent.
    pub transform_request_body: Option<TransformRequestBody>,
}

impl OpenResponsesProviderSettings {
    /// Settings with the two required fields; the rest unset.
    #[must_use]
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: None,
            headers: None,
            fetch: None,
            transform_request_body: None,
        }
    }
}

impl std::fmt::Debug for OpenResponsesProviderSettings {
    /// Never prints the key or header values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenResponsesProviderSettings")
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key)
            .field("headers", &self.headers.is_some())
            .field("fetch", &self.fetch.is_some())
            .field(
                "transform_request_body",
                &self.transform_request_body.is_some(),
            )
            .finish()
    }
}

/// The first dot-separated segment of a provider name, trimmed: the
/// providerOptions key of the provider.
fn options_name_of(name: &str) -> String {
    name.split('.')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Create an Open Responses provider.
///
/// # Errors
///
/// Returns `AiMuxError::InvalidArgument` when `name` is empty or `base_url` is
/// not an `http(s)` URL with a host. Credentials are resolved per request, not
/// here.
pub fn create_open_responses(
    settings: OpenResponsesProviderSettings,
) -> Result<OpenResponsesProvider, AiMuxError> {
    if settings.name.trim().is_empty() {
        return Err(AiMuxError::InvalidArgument(
            "Open Responses requires a non-empty `name`.".to_string(),
        ));
    }
    let base_url = validate_base_url(&settings.base_url)?;
    let credential = match settings.api_key {
        Some(key) => Credential::Explicit(key),
        None => Credential::None,
    };
    let provider_layer = provider_headers(credential, Vec::new(), None);
    let user = settings.headers;
    let headers: HeadersFn = Resolvable::from_async_fn(move || {
        let provider_layer = provider_layer.clone();
        let user = user.clone();
        async move {
            let layer = provider_layer.resolve().await?;
            match &user {
                Some(user) => Ok(combine_headers(&[&layer, &user.resolve().await?])),
                None => Ok(layer),
            }
        }
    });
    Ok(OpenResponsesProvider {
        name: settings.name,
        base_url,
        headers,
        fetch: settings.fetch,
        transform_request_body: settings.transform_request_body,
    })
}

// == Provider ==

/// Open Responses provider - creates [`OpenResponsesModel`] instances.
pub struct OpenResponsesProvider {
    name: String,
    base_url: String,
    headers: HeadersFn,
    fetch: Option<FetchFunction>,
    transform_request_body: Option<TransformRequestBody>,
}

impl OpenResponsesProvider {
    /// A Responses model; `provider()` is `"{name}.responses"`.
    #[must_use]
    pub fn responses(&self, model_id: &str) -> OpenResponsesModel {
        OpenResponsesModel {
            model_id: model_id.to_string(),
            provider_options_name: options_name_of(&self.name),
            config: EndpointConfig::fixed(
                format!("{}.responses", self.name),
                self.base_url.clone(),
                self.headers.clone(),
                self.fetch.clone(),
                self.transform_request_body.clone(),
            ),
        }
    }

    /// The provider as a function: the default language model for an id. The
    /// same model as [`responses`](Self::responses) and
    /// [`language_model`](Provider::language_model).
    #[must_use]
    pub fn call(&self, model_id: &str) -> Arc<dyn LanguageModel> {
        Arc::new(self.responses(model_id))
    }
}

impl Provider for OpenResponsesProvider {
    fn language_model(&self, model_id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(self.call(model_id))
    }

    fn embedding_model(&self, model_id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "embeddingModel"))
    }

    fn image_model(&self, model_id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(model_id, "imageModel"))
    }
}

// == Model ==

/// An Open Responses language model.
pub struct OpenResponsesModel {
    model_id: String,
    /// The providerOptions key read (first segment of the provider name).
    provider_options_name: String,
    config: EndpointConfig,
}

#[async_trait]
impl LanguageModel for OpenResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        validate_tool_output_media(&options.prompt)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
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
                                    content.push(GenerateContent::Reasoning(ReasoningOutput {
                                        text: text.to_string(),
                                        provider_metadata: None,
                                    }));
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
                        content.push(GenerateContent::ToolCall(RawToolCall {
                            tool_call_id: call_id,
                            tool_name: name,
                            input,
                            provider_executed: None,
                            dynamic: None,
                            provider_metadata: None,
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
            response: Some(aimux_core::shared::ResponseInfo {
                headers: Some(response_headers),
                body: Some(raw),
                ..response.into()
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        validate_tool_output_media(&options.prompt)?;
        let exchange = self.config.exchange(options.headers.as_ref()).await?;
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
            return Err(aimux_provider_utils::stream_error_api_call(
                message,
                provider_code,
                status_code,
                event,
                endpoint,
                stream_body,
                response_headers.clone(),
            ));
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
                    Ok(chunk) => {
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
                                            let input = arguments;
                                            yield Ok(StreamPart::ToolCall(RawToolCall {
                                                tool_call_id,
                                                tool_name,
                                                input,
                                                provider_executed: None,
                                                dynamic: None,
                                                provider_metadata: None,
                                            }));
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
                    Err(error) => {
                        let recoverable = error.is_recoverable_stream_error();
                        yield Err(error);
                        if !recoverable {
                            return;
                        }
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
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::shared::StreamResponseInfo {
                headers: Some(response_headers),
            }),
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
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let converted_tool_choice: Option<Value> = match &options.tool_choice {
        None => None,
        Some(ToolChoice::Auto) => Some(json!("auto")),
        Some(ToolChoice::None) => Some(json!("none")),
        Some(ToolChoice::Required) => Some(json!("required")),
        Some(ToolChoice::Tool { tool_name }) => Some(json!({
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
    convert_to_open_responses_input_with_namespace(prompt, "open-responses")
}

fn convert_to_open_responses_input_with_namespace(
    prompt: &LanguageModelPrompt,
    provider_options_name: &str,
) -> (Value, Option<String>, Vec<Warning>) {
    let mut input: Vec<Value> = Vec::new();
    let mut warnings = Vec::new();
    let mut system_messages: Vec<String> = Vec::new();

    for msg in prompt {
        match msg {
            LanguageModelMessage::System { content, .. } => {
                system_messages.push(content.clone());
            }
            LanguageModelMessage::User { content, .. } => {
                let user_content = convert_user_content(content, &mut warnings);
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": user_content,
                }));
            }
            LanguageModelMessage::Assistant { content, .. } => {
                let mut assistant_content: Vec<Value> = Vec::new();

                for part in content {
                    match part {
                        AssistantPart::Text(TextPart { text, .. }) => {
                            assistant_content.push(json!({
                                "type": "output_text",
                                "text": text,
                            }));
                        }
                        AssistantPart::ToolCall(ToolCallPart {
                            tool_call_id,
                            tool_name,
                            input: tool_input,
                            ..
                        }) => {
                            let arguments = match tool_input {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            flush_assistant_content(&mut assistant_content, &mut input);
                            input.push(json!({
                                "type": "function_call",
                                "call_id": tool_call_id,
                                "name": tool_name,
                                "arguments": arguments,
                            }));
                        }
                        AssistantPart::Reasoning(part) => {
                            flush_assistant_content(&mut assistant_content, &mut input);
                            let metadata = part
                                .provider_options
                                .as_ref()
                                .and_then(|options| options.get(provider_options_name));
                            let parse_parts = |key: &str, kind: &str| {
                                metadata
                                    .and_then(|data| data.get(key))
                                    .and_then(Value::as_array)
                                    .filter(|parts| {
                                        parts.iter().all(|part| {
                                            part["type"] == kind && part["text"].is_string()
                                        })
                                    })
                                    .map(|parts| {
                                        json!(
                                            parts
                                                .iter()
                                                .map(
                                                    |part| json!({"type":kind, "text":part["text"]})
                                                )
                                                .collect::<Vec<_>>()
                                        )
                                    })
                            };
                            let mut reasoning = json!({"type":"reasoning", "summary":parse_parts("reasoningSummary", "summary_text").unwrap_or_else(|| json!([]))});
                            if let Some(id) = metadata
                                .and_then(|data| data.get("itemId"))
                                .and_then(Value::as_str)
                            {
                                reasoning["id"] = json!(id);
                            }
                            if let Some(content) = parse_parts("reasoningContent", "reasoning_text")
                            {
                                reasoning["content"] = content;
                            } else if !metadata
                                .is_some_and(|data| data.contains_key("reasoningContent"))
                                && !part.text.is_empty()
                            {
                                reasoning["content"] =
                                    json!([{"type":"reasoning_text", "text":part.text}]);
                            }
                            if let Some(encrypted) = metadata
                                .and_then(|data| data.get("reasoningEncryptedContent"))
                                .and_then(Value::as_str)
                            {
                                reasoning["encrypted_content"] = json!(encrypted);
                            }
                            if let Some(previous) = input.last_mut().filter(|previous| {
                                reasoning.get("id").is_some()
                                    && previous["type"] == "reasoning"
                                    && previous.get("id") == reasoning.get("id")
                            }) {
                                if let Some(content) =
                                    reasoning.get("content").and_then(Value::as_array)
                                {
                                    if previous.get("content").is_none() {
                                        previous["content"] = json!([]);
                                    }
                                    if let Some(previous_content) =
                                        previous["content"].as_array_mut()
                                    {
                                        previous_content.extend(content.iter().cloned());
                                    }
                                }
                            } else {
                                input.push(reasoning);
                            }
                        }
                        _ => {}
                    }
                }
                flush_assistant_content(&mut assistant_content, &mut input);
            }
            LanguageModelMessage::Tool { content, .. } => {
                for part in content {
                    let ToolPart::ToolResult(ToolResultPart {
                        tool_call_id,
                        output,
                        ..
                    }) = part
                    else {
                        continue;
                    };
                    let content_value =
                        resolve_tool_result_output(output, provider_options_name, &mut warnings);
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_call_id,
                        "output": content_value,
                    }));
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

fn flush_assistant_content(content: &mut Vec<Value>, input: &mut Vec<Value>) {
    if !content.is_empty() {
        input
            .push(json!({"type":"message", "role":"assistant", "content":std::mem::take(content)}));
    }
}

/// Convert user message content parts to the Open Responses format.
fn convert_user_content(content: &[UserPart], warnings: &mut Vec<Warning>) -> Value {
    use base64::Engine;

    let mut parts: Vec<Value> = Vec::new();
    for part in content {
        match part {
            UserPart::Text(TextPart { text, .. }) => {
                parts.push(json!({ "type": "input_text", "text": text }));
            }
            UserPart::File(FilePart {
                data,
                media_type,
                filename,
                ..
            }) => {
                let image = top_level_media_type(media_type) == "image";
                match data {
                    FileData::Data { data } => {
                        let b64 = match data {
                            FileBytes::Binary(bytes) => {
                                base64::engine::general_purpose::STANDARD.encode(bytes)
                            }
                            FileBytes::Base64(data) => data.clone(),
                        };
                        let data_url = format!("data:{media_type};base64,{b64}");
                        parts.push(if image {
                            json!({ "type": "input_image", "image_url": data_url })
                        } else {
                            json!({
                                "type": "input_file",
                                "filename": filename.as_deref().unwrap_or("data"),
                                "file_data": data_url,
                            })
                        });
                    }
                    FileData::Url { url, .. } => {
                        parts.push(if image {
                            json!({ "type": "input_image", "image_url": url })
                        } else {
                            json!({ "type": "input_file", "file_url": url })
                        });
                    }
                    FileData::Reference { .. } => {
                        warnings.push(Warning::Other {
                            message: "unsupported file part with provider reference".to_string(),
                        });
                    }
                    FileData::Text { .. } => {
                        warnings.push(Warning::Other {
                            message: "unsupported text file part".to_string(),
                        });
                    }
                }
            }
        }
    }
    json!(parts)
}

fn validate_tool_output_media(prompt: &LanguageModelPrompt) -> Result<(), AiMuxError> {
    for message in prompt {
        let LanguageModelMessage::Tool { content, .. } = message else {
            continue;
        };
        for part in content {
            let ToolPart::ToolResult(ToolResultPart {
                output: ToolResultOutput::Content { value },
                ..
            }) = part
            else {
                continue;
            };
            for item in value {
                if let ToolResultContent::File(part) = item
                    && matches!(part.data, FileData::Data { .. })
                {
                    aimux_provider_utils::resolve_full_media_type(part)?;
                }
            }
        }
    }
    Ok(())
}

/// Resolve a tool-result `output` value into the Open Responses `output`
/// field, mirroring the TS convert logic.
fn resolve_tool_result_output(
    output: &ToolResultOutput,
    provider_options_name: &str,
    warnings: &mut Vec<Warning>,
) -> Value {
    let ToolResultOutput::Content { value } = output else {
        return crate::openai::convert::tool_result_to_content(output);
    };
    let mut parts = Vec::new();
    for item in value {
        match item {
            ToolResultContent::Text(part) => {
                parts.push(json!({"type":"input_text", "text":part.text}))
            }
            ToolResultContent::File(part) => {
                let image = top_level_media_type(&part.media_type) == "image";
                let mut converted = match &part.data {
                    FileData::Data { data } => {
                        use base64::Engine;
                        let media_type = match aimux_provider_utils::resolve_full_media_type(part) {
                            Ok(media_type) => media_type,
                            Err(error) => {
                                warnings.push(Warning::Other {
                                    message: error.to_string(),
                                });
                                continue;
                            }
                        };
                        let b64 = match data {
                            FileBytes::Binary(bytes) => {
                                base64::engine::general_purpose::STANDARD.encode(bytes)
                            }
                            FileBytes::Base64(data) => data.clone(),
                        };
                        let data_url = format!("data:{media_type};base64,{b64}");
                        if image {
                            json!({"type":"input_image", "image_url":data_url})
                        } else {
                            json!({"type":"input_file", "filename":part.filename.as_deref().unwrap_or("data"), "file_data":data_url})
                        }
                    }
                    FileData::Url { url, .. } => {
                        if image {
                            json!({"type":"input_image", "image_url":url})
                        } else {
                            json!({"type":"input_file", "file_url":url})
                        }
                    }
                    data => {
                        warnings.push(Warning::Other {
                            message: format!(
                                "unsupported tool content part type: file with data type: {}",
                                if matches!(data, FileData::Reference { .. }) {
                                    "reference"
                                } else {
                                    "text"
                                }
                            ),
                        });
                        continue;
                    }
                };
                if image {
                    let detail = part
                        .provider_options
                        .as_ref()
                        .and_then(|options| options.get(provider_options_name))
                        .and_then(|options| options.get("imageDetail"))
                        .and_then(Value::as_str)
                        .filter(|detail| matches!(*detail, "low" | "high" | "auto"))
                        .unwrap_or("auto");
                    converted["detail"] = json!(detail);
                }
                parts.push(converted);
            }
            ToolResultContent::Custom { .. } => warnings.push(Warning::Other {
                message: "unsupported tool content part type: custom".into(),
            }),
        }
    }
    json!(parts)
}

/// Extract the top-level media type (e.g. "image" from "image/png").
fn top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
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
