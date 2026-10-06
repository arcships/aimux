//! The OpenAI-compatible chat language model (`{name}.chat`).
//!
//! The Rust form of `OpenAICompatibleChatLanguageModel`. Request construction
//! lives in [`super::convert`]; vendor differences arrive through the
//! [`ChatDialect`](super::config::ChatDialect) of the model's config.

use aimux_core::tool::RawToolCall;
use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, ReasoningOutput, Source, StreamResult};
use aimux_core::shared::provider_namespace;
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, InputTokenUsage, OutputTokenUsage, ResponseMetadata, Usage,
};
use aimux_provider_utils::{
    StreamingToolCallDelta, StreamingToolCallFunction, StreamingToolCallTracker, generate_id,
};

use super::config::CompatModelConfig;
use super::convert::{ChatBodySpec, RequestBodyResult, build_request_body, parse_finish_reason};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// An OpenAI-compatible chat model. Holds no HTTP client: the request helpers
/// of `aimux-provider-utils` use the injected or process-default transport.
pub struct OpenAICompatibleChatModel {
    model_id: String,
    config: CompatModelConfig,
}

impl OpenAICompatibleChatModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
    }

    /// The JSON body a call would send, before the provider's
    /// `transform_request_body`, with the warnings raised while building it.
    /// Nothing is read from the environment and nothing is sent: a key is not
    /// resolved here.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a malformed provider option or an unconvertible
    /// prompt part.
    pub(crate) fn request_body(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<RequestBodyResult, AiMuxError> {
        build_request_body(
            &self.model_id,
            options,
            stream,
            &ChatBodySpec {
                provider_options_name: self.config.provider_options_name(),
                include_usage: self.config.chat.include_usage,
                supports_structured_outputs: self.config.chat.supports_structured_outputs,
                supports_multi_part_tool_content: self.config.chat.supports_multi_part_tool_content,
                dialect: &self.config.chat.dialect,
            },
        )
    }
}

// ── Usage ────────────────────────────────────────────────────────────────────

/// Core usage from a parsed OpenAI-shaped `usage` object.
///
/// `raw` is the provider's original object, kept verbatim in `Usage.raw`.
/// `cached_tokens` at the top level (Moonshot) wins over the nested value.
pub(crate) fn convert_usage(usage: &UsageResponse, raw: Option<&Value>) -> Usage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);

    let nested_cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    let cached = usage.cached_tokens.unwrap_or(nested_cached);
    let cache_write = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cache_write_tokens);

    // Saturating: some servers report more cached or reasoning tokens than
    // prompt or completion tokens.
    let no_cache = prompt_tokens
        .saturating_sub(cached)
        .saturating_sub(cache_write.unwrap_or(0));
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);

    Usage {
        input_tokens: InputTokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached),
            cache_write,
        },
        output_tokens: OutputTokenUsage {
            total: Some(completion_tokens),
            text: Some(completion_tokens.saturating_sub(reasoning_tokens)),
            reasoning: Some(reasoning_tokens),
        },
        raw: raw.and_then(|value| value.as_object().cloned()),
    }
}

/// Usage from a raw `usage` value.
pub(crate) fn usage_from_raw(raw: Option<&Value>) -> Usage {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Usage::default();
    };
    match serde_json::from_value::<UsageResponse>(raw.clone()) {
        Ok(parsed) => convert_usage(&parsed, Some(raw)),
        Err(_) => Usage {
            raw: raw.as_object().cloned(),
            ..Usage::default()
        },
    }
}

fn prediction_tokens(usage: Option<&UsageResponse>, metadata: &mut Map<String, Value>) {
    if let Some(details) = usage.and_then(|u| u.completion_tokens_details.as_ref()) {
        if let Some(accepted) = details.accepted_prediction_tokens {
            metadata.insert("acceptedPredictionTokens".into(), json!(accepted));
        }
        if let Some(rejected) = details.rejected_prediction_tokens {
            metadata.insert("rejectedPredictionTokens".into(), json!(rejected));
        }
    }
}

fn thought_signature(extra_content: Option<&Value>) -> Option<String> {
    extra_content
        .and_then(|extra| extra.pointer("/google/thought_signature"))
        .and_then(Value::as_str)
        .filter(|signature| !signature.is_empty())
        .map(str::to_string)
}

fn stream_error(
    name: &str,
    error: &Value,
    url: &str,
    request_body_values: Value,
    response_headers: HashMap<String, String>,
) -> AiMuxError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{name} stream failed before any output was generated"));
    let code = error.get("code").or_else(|| error.get("type"));
    let provider_code = code.and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    });
    // Only a numeric HTTP status in the payload is a status; a string code
    // must not be laundered into a retryable 500.
    let status_code = code
        .and_then(Value::as_u64)
        .filter(|status| (400..=599).contains(status))
        .map(|status| status as u16);
    aimux_provider_utils::stream_error_api_call(
        message,
        provider_code,
        status_code,
        error,
        url,
        request_body_values,
        response_headers,
    )
}

// ── LanguageModel ────────────────────────────────────────────────────────────

#[async_trait]
impl LanguageModel for OpenAICompatibleChatModel {
    /// `"{name}.chat"`.
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn supported_urls(&self) -> SupportedUrls {
        self.config
            .chat
            .supported_urls
            .as_ref()
            .map(|urls| urls())
            .unwrap_or_default()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let built = self.request_body(options, false)?;
        let metadata_key = built.metadata_key;
        let body = self.config.transform_body(built.body);
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let http = self
            .config
            .http_request("/chat/completions", headers, options)?;

        let resp = aimux_provider_utils::post_json_to_api(
            http,
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<ChatCompletionResponse>(),
            self.config.failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw = resp.raw_value.unwrap_or(Value::Null);
        let data = resp.value;

        let choice = data.choices.into_iter().next().ok_or_else(|| {
            AiMuxError::InvalidResponseData("Response did not contain any choices.".to_string())
        })?;

        let mut content = Vec::new();
        for text in content_texts(choice.message.content.as_ref()) {
            content.push(GenerateContent::Text {
                text,
                provider_metadata: None,
            });
        }
        let reasoning = choice
            .message
            .reasoning_content
            .or(choice.message.reasoning)
            .filter(|text| !text.is_empty());
        if let Some(text) = reasoning {
            content.push(GenerateContent::Reasoning(ReasoningOutput {
                text,
                provider_metadata: None,
            }));
        }
        for call in choice.message.tool_calls.into_iter().flatten() {
            let signature = thought_signature(call.extra_content.as_ref());
            content.push(GenerateContent::ToolCall(RawToolCall {
                tool_call_id: call
                    .id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(generate_id),
                tool_name: call.function.name,
                input: call.function.arguments.unwrap_or_default(),
                provider_executed: None,
                dynamic: None,
                provider_metadata: signature.as_ref().map(|signature| {
                    provider_namespace(&metadata_key, json!({ "thoughtSignature": signature }))
                        .expect("provider metadata must be an object")
                }),
            }));
        }
        for (i, annotation) in choice.message.annotations.iter().flatten().enumerate() {
            if annotation.get("type").and_then(Value::as_str) == Some("url_citation")
                && let Some(citation) = annotation.get("url_citation")
            {
                content.push(GenerateContent::Source(Source::Url {
                    id: format!("annotation-{i}"),
                    url: citation
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    title: citation
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    provider_metadata: None,
                }));
            }
        }

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });

        let dialect = &self.config.chat.dialect;
        let usage = self
            .config
            .chat
            .convert_usage
            .as_ref()
            .map(|convert| convert(raw.get("usage").unwrap_or(&Value::Null)))
            .unwrap_or_else(|| usage_from_raw(raw.get("usage")));

        let mut namespace = Map::new();
        prediction_tokens(data.usage.as_ref(), &mut namespace);
        let mut metadata = HashMap::new();
        metadata.insert(metadata_key, namespace);
        if let Some(extractor) = &dialect.metadata_extractor
            && let Some(extra) = extractor.extract_metadata(&raw)
        {
            for (key, value) in extra {
                metadata.insert(key, value);
            }
        }

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: built.warnings,
            provider_metadata: Some(metadata),
            response: Some(aimux_core::shared::ResponseInfo {
                id: data.id,
                timestamp: data
                    .created
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                    .map(|dt| dt.to_rfc3339()),
                model_id: data.model,
                headers: Some(response_headers),
                body: Some(raw),
            }),
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let built = self.request_body(options, true)?;
        let metadata_key = built.metadata_key;
        let warnings = built.warnings;
        let body = self.config.transform_body(built.body);
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let http = self
            .config
            .http_request("/chat/completions", headers, options)?;
        let endpoint = http.url.clone();

        let resp = aimux_provider_utils::post_json_to_api(
            http,
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            self.config.failed_response_handler(),
        )
        .await?;
        let response_headers = resp.response_headers;
        let mut sse_stream = resp.value;

        // Only the first event is checked before returning the stream: an
        // error reported immediately stays inside Core's operation-retry
        // boundary. A normal event is chained back and never consumed.
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first => first,
        };
        if let Some(Ok(event)) = &first_event
            && let Some(error) = event.get("error")
        {
            return Err(stream_error(
                &metadata_key,
                error,
                &endpoint,
                body.clone(),
                response_headers.clone(),
            ));
        }

        let dialect = self.config.chat.dialect.clone();
        let mut metadata_extractor = dialect
            .metadata_extractor
            .as_ref()
            .map(|extractor| extractor.create_stream_extractor());
        let convert_usage = self.config.chat.convert_usage.clone();
        let emit_raw_chunks = options.include_raw_chunks == Some(true);
        let stream_error_url = endpoint;
        let stream_error_body = body.clone();
        let stream_response_headers = response_headers.clone();

        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });

            let text_id = 0usize;
            let mut text_started = false;
            let reasoning_id = "reasoning-0".to_string();
            let mut reasoning_started = false;
            let mut final_usage_raw: Option<Value> = None;
            let mut final_usage_parsed: Option<UsageResponse> = None;
            let mut final_finish_reason: Option<FinishReason> = None;
            let mut response_metadata_emitted = false;

            let signature_key = metadata_key.clone();
            let mut tool_calls = StreamingToolCallTracker::new()
                .with_generate_id(generate_id)
                .with_extract_metadata(|delta| {
                    thought_signature(Some(&delta.extra)).map(Value::String)
                })
                .with_build_provider_metadata(move |signature| {
                    signature
                        .and_then(Value::as_str)
                        .map(|signature| provider_namespace(&signature_key, json!({ "thoughtSignature": signature })).expect("provider metadata must be an object"))
                });

            // Some compatible servers send the first delta of a call without
            // `function.name`; buffer by index until the name is known.
            let mut pending: HashMap<usize, PendingToolCall> = HashMap::new();
            let mut forwarded: HashSet<usize> = HashSet::new();

            let mut event_iter =
                futures::stream::iter(first_event.into_iter()).chain(sse_stream);
            let mut stream_errored = false;

            while let Some(event) = event_iter.next().await {
                if stream_errored {
                    break;
                }
                match event {
                    Ok(parsed) => {
                        if emit_raw_chunks {
                            yield Ok(StreamPart::Raw { raw_value: parsed.clone() });
                        }
                        if let Some(extractor) = metadata_extractor.as_mut() {
                            extractor.process_chunk(&parsed);
                        }
                        if let Some(error) = parsed.get("error") {
                            yield Ok(StreamPart::Error {
                                error: stream_error(
                                    &metadata_key,
                                    error,
                                    &stream_error_url,
                                    stream_error_body.clone(),
                                    stream_response_headers.clone(),
                                ),
                            });
                            stream_errored = true;
                            break;
                        }

                        let chunk_usage_raw: Option<Value> = match &dialect.stream_usage_key {
                            Some(key) => parsed.get(key).and_then(|v| v.get("usage")).cloned(),
                            None => parsed.get("usage").cloned(),
                        }
                        .filter(|usage| !usage.is_null());

                        let chunk: StreamChunk = match serde_json::from_value(parsed) {
                            Ok(chunk) => chunk,
                            Err(e) => {
                                yield Err(e.into());
                                continue;
                            }
                        };

                        if !response_metadata_emitted
                            && (chunk.id.is_some() || chunk.model.is_some())
                        {
                            response_metadata_emitted = true;
                            yield Ok(StreamPart::ResponseMetadata(ResponseMetadata {
                                id: chunk.id.clone(),
                                timestamp: chunk
                                    .created
                                    .and_then(|secs| chrono::DateTime::from_timestamp(secs as i64, 0))
                                    .map(|dt| dt.to_rfc3339()),
                                model_id: chunk.model.clone(),
                            }));
                        }

                        if let Some(raw_usage) = &chunk_usage_raw {
                            final_usage_raw = Some(raw_usage.clone());
                            final_usage_parsed = serde_json::from_value(raw_usage.clone()).ok();
                        }

                        for choice in chunk.choices {
                            let reasoning_delta = choice
                                .delta
                                .reasoning_content
                                .clone()
                                .or_else(|| choice.delta.reasoning.clone());
                            if let Some(reasoning) = reasoning_delta
                                && !reasoning.is_empty()
                            {
                                if !reasoning_started {
                                    reasoning_started = true;
                                    yield Ok(StreamPart::ReasoningStart {
                                        id: reasoning_id.clone(),
                                        provider_metadata: None,
                                    });
                                }
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: reasoning_id.clone(),
                                    delta: reasoning,
                                    provider_metadata: None,
                                });
                            }

                            if let Some(content) = choice.delta.content {
                                if reasoning_started {
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: reasoning_id.clone(),
                                        provider_metadata: None,
                                    });
                                    reasoning_started = false;
                                }
                                if !text_started {
                                    text_started = true;
                                    yield Ok(StreamPart::TextStart {
                                        id: format!("{text_id}"),
                                        provider_metadata: None,
                                    });
                                }
                                yield Ok(StreamPart::TextDelta {
                                    id: format!("{text_id}"),
                                    delta: content,
                                    provider_metadata: None,
                                });
                            }

                            if let Some(deltas) = choice.delta.tool_calls {
                                if reasoning_started {
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: reasoning_id.clone(),
                                        provider_metadata: None,
                                    });
                                    reasoning_started = false;
                                }
                                for dtc in deltas {
                                    let name = dtc
                                        .function
                                        .as_ref()
                                        .and_then(|f| f.name.clone())
                                        .filter(|n| !n.trim().is_empty());
                                    let arguments =
                                        dtc.function.as_ref().and_then(|f| f.arguments.clone());
                                    let extra = dtc.extra_content.clone().unwrap_or(Value::Null);
                                    let delta = match dtc.index {
                                        Some(index) if !forwarded.contains(&index) => {
                                            let entry = pending.entry(index).or_default();
                                            if entry.id.is_none() {
                                                entry.id = dtc.id.clone();
                                            }
                                            if entry.extra.is_null() {
                                                entry.extra = extra;
                                            }
                                            if let Some(arguments) = &arguments {
                                                entry.arguments.push_str(arguments);
                                            }
                                            let Some(name) = name else { continue };
                                            let entry = pending.remove(&index).unwrap_or_default();
                                            forwarded.insert(index);
                                            StreamingToolCallDelta {
                                                index: Some(index),
                                                id: entry.id,
                                                r#type: None,
                                                function: Some(StreamingToolCallFunction {
                                                    name: Some(name),
                                                    arguments: Some(entry.arguments),
                                                }),
                                                extra: entry.extra,
                                            }
                                        }
                                        _ => StreamingToolCallDelta {
                                            index: dtc.index,
                                            id: dtc.id.clone(),
                                            r#type: None,
                                            function: dtc.function.as_ref().map(|f| {
                                                StreamingToolCallFunction {
                                                    name: f.name.clone(),
                                                    arguments: f.arguments.clone(),
                                                }
                                            }),
                                            extra,
                                        },
                                    };
                                    match tool_calls.process_delta(&delta) {
                                        Ok(parts) => {
                                            for part in parts {
                                                yield Ok(part);
                                            }
                                        }
                                        Err(error) => {
                                            yield Err(error.into());
                                            return;
                                        }
                                    }
                                }
                            }

                            if let Some(annotations) = choice.delta.annotations {
                                for (i, annotation) in annotations.iter().enumerate() {
                                    if annotation.get("type").and_then(Value::as_str)
                                        == Some("url_citation")
                                        && let Some(citation) = annotation.get("url_citation")
                                    {
                                        yield Ok(StreamPart::Source(Source::Url {
                                            id: format!("annotation-{i}"),
                                            url: citation
                                                .get("url")
                                                .and_then(Value::as_str)
                                                .unwrap_or_default()
                                                .to_string(),
                                            title: citation
                                                .get("title")
                                                .and_then(Value::as_str)
                                                .map(str::to_string),
                                            provider_metadata: None,
                                        }));
                                    }
                                }
                            }

                            if let Some(reason) = choice.finish_reason {
                                if reasoning_started {
                                    yield Ok(StreamPart::ReasoningEnd {
                                        id: reasoning_id.clone(),
                                        provider_metadata: None,
                                    });
                                    reasoning_started = false;
                                }
                                if text_started {
                                    yield Ok(StreamPart::TextEnd {
                                        id: format!("{text_id}"),
                                        provider_metadata: None,
                                    });
                                    text_started = false;
                                }
                                final_finish_reason = Some(parse_finish_reason(&reason));
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

            if reasoning_started {
                yield Ok(StreamPart::ReasoningEnd {
                    id: reasoning_id.clone(),
                    provider_metadata: None,
                });
            }
            if text_started {
                yield Ok(StreamPart::TextEnd {
                    id: format!("{text_id}"),
                    provider_metadata: None,
                });
            }
            // A parsable argument buffer can still be a prefix of a longer
            // input: finalize only when the stream flushes.
            for part in tool_calls.flush() {
                yield Ok(part);
            }

            let mut namespace = Map::new();
            prediction_tokens(final_usage_parsed.as_ref(), &mut namespace);
            let mut metadata = HashMap::new();
            metadata.insert(metadata_key.clone(), namespace);
            if let Some(extractor) = &metadata_extractor
                && let Some(extra) = extractor.build_metadata()
            {
                for (key, value) in extra {
                    metadata.insert(key, value);
                }
            }

            yield Ok(StreamPart::Finish {
                finish_reason: if stream_errored {
                    FinishReason { unified: FinishReasonUnified::Error, raw: None }
                } else {
                    final_finish_reason.unwrap_or(FinishReason {
                        unified: FinishReasonUnified::Stop,
                        raw: None,
                    })
                },
                usage: if stream_errored { Usage::default() } else {
                    convert_usage.as_ref().map(|convert| convert(final_usage_raw.as_ref().unwrap_or(&Value::Null)))
                        .unwrap_or_else(|| usage_from_raw(final_usage_raw.as_ref()))
                },
                provider_metadata: Some(metadata),
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

#[derive(Default)]
struct PendingToolCall {
    id: Option<String>,
    arguments: String,
    extra: Value,
}

/// Text segments of a message `content`: a string, or an array of
/// `{ type: "text", text }` parts.
fn content_texts(content: Option<&Value>) -> Vec<String> {
    match content {
        Some(Value::String(text)) if !text.is_empty() => vec![text.clone()],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// One `GET {base_url}/models` exchange for discovery: no retry, no
/// recording.
pub(crate) async fn list_models_once(
    config: &CompatModelConfig,
) -> Result<Vec<aimux_core::model_catalogue::RuntimeModel>, AiMuxError> {
    #[derive(serde::Deserialize)]
    struct ModelsList {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
        #[serde(default)]
        owned_by: Option<String>,
        #[serde(default)]
        created: Option<u64>,
    }

    let headers = config.request_headers(None).await?;
    let (url, origin) = config.url("/models")?;
    let resp = aimux_provider_utils::get_from_api(
        config.with_transport(
            aimux_provider_utils::HttpRequest {
                url,
                headers,
                ..Default::default()
            },
            origin,
        ),
        aimux_provider_utils::create_json_response_handler::<ModelsList>(),
        config.failed_response_handler(),
    )
    .await?;
    Ok(resp
        .value
        .data
        .into_iter()
        .map(|m| aimux_core::model_catalogue::RuntimeModel {
            id: m.id,
            owned_by: m.owned_by,
            created: m.created,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use aimux_core::error::AiMuxError;
    use aimux_core::language_model::LanguageModel;
    use aimux_core::language_model_message::LanguageModelMessage;
    use aimux_core::options::CallOptions;
    use aimux_provider_utils::ProviderErrorParts;

    use crate::openai_compatible::config::{BaseUrl, ChatDialect};
    use crate::openai_compatible::{Assembly, ChatProfile, OpenAICompatibleProvider};
    use crate::shared::Credential;

    fn hello() -> CallOptions {
        CallOptions::new(vec![LanguageModelMessage::user_text("hi")])
    }

    fn provider(base_url: BaseUrl, dialect: ChatDialect) -> OpenAICompatibleProvider {
        OpenAICompatibleProvider::assemble(Assembly {
            name: "acme".to_string(),
            base_url,
            credential: Credential::None,
            fixed_headers: Vec::new(),
            headers: None,
            query_params: None,
            fetch: None,
            transform_request_body: None,
            profile: ChatProfile {
                include_usage: false,
                supports_structured_outputs: false,
                supports_multi_part_tool_content: false,
                dialect,
                supported_urls: None,
                convert_usage: None,
            },
        })
        .unwrap()
    }

    async fn serve(server: &MockServer, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(response)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn an_error_structure_decides_the_message_and_code() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(400).set_body_json(json!({"detail": "bad prompt", "kind": 42})),
        )
        .await;
        let mut dialect = ChatDialect::baseline();
        dialect.error_structure = Arc::new(|data: &Value| ProviderErrorParts {
            message: data["detail"].as_str().unwrap_or_default().to_string(),
            provider_code: data["kind"].as_u64().map(|k| k.to_string()),
        });
        let error = provider(BaseUrl::Fixed(server.uri()), dialect)
            .chat("m")
            .do_generate(&hello())
            .await
            .unwrap_err();
        match error {
            AiMuxError::ApiCall(detail) => {
                assert_eq!(detail.message, "bad prompt");
                assert_eq!(detail.provider_code.as_deref(), Some("42"));
            }
            other => panic!("expected ApiCall, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_lazy_base_url_is_evaluated_per_request_and_its_error_fails_the_request() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "c", "model": "m",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
            })),
        )
        .await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let uri = server.uri();
        let lazy = BaseUrl::Lazy(Arc::new(move || {
            if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Err(AiMuxError::InvalidArgument("not ready".into()))
            } else {
                Ok(uri.clone())
            }
        }));
        let provider = provider(lazy, ChatDialect::baseline());
        let model = provider.chat("m");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "creating a provider or a model resolves nothing"
        );
        let error = model.do_generate(&hello()).await.unwrap_err();
        assert!(matches!(error, AiMuxError::InvalidArgument(_)), "{error:?}");
        model
            .do_generate(&hello())
            .await
            .expect("resolved on the next request");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
