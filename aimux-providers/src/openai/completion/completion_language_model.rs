//! The legacy text-completion model (`{name}.completion`,
//! openai-completion-language-model.ts), with its prompt
//! (convert-to-openai-completion-prompt.ts), usage
//! (convert-openai-completion-usage.ts), response metadata
//! (get-response-metadata.ts), finish reason (map-openai-finish-reason.ts),
//! response schema (openai-completion-api.ts) and provider options
//! (openai-completion-language-model-options.ts).

use aimux_core::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage, UserPart};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::{
    RequestInfo, ResponseInfo, SharedProviderMetadata, StreamResponseInfo, provider_namespace,
};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, InputTokenUsage, OutputTokenUsage, ResponseMetadata, Usage, Warning,
};
use aimux_provider_utils::{HttpRequest, Transformer, pipe_through};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::openai::chat::map_finish_reason::parse_finish_reason;
use crate::openai::config::OpenAIModelConfig;
use crate::openai::{openai_failed_response_handler, openai_stream_error};
use crate::stream::{Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext};

/// The native provider's `/completions` model.
#[derive(Clone)]
pub struct OpenAICompletionModel {
    model_id: String,
    config: OpenAIModelConfig,
}

impl OpenAICompletionModel {
    pub(crate) fn from_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self { model_id, config }
    }

    /// `providerOptionsName`: the provider name up to the first dot.
    fn provider_options_name(&self) -> &str {
        self.config
            .provider
            .split('.')
            .next()
            .unwrap_or_default()
            .trim()
    }

    fn arguments(&self, options: &CallOptions) -> Result<(Value, Vec<Warning>), AiMuxError> {
        let mut warnings = Vec::new();
        for (present, feature) in [
            (options.top_k.is_some(), "topK"),
            (
                options
                    .tools
                    .as_ref()
                    .is_some_and(|tools| !tools.is_empty()),
                "tools",
            ),
            (options.tool_choice.is_some(), "toolChoice"),
        ] {
            if present {
                warnings.push(Warning::Unsupported {
                    feature: feature.into(),
                    details: None,
                });
            }
        }
        if matches!(options.response_format, Some(ResponseFormat::Json { .. })) {
            warnings.push(Warning::Unsupported {
                feature: "responseFormat".into(),
                details: Some("JSON response format is not supported.".into()),
            });
        }
        let mut provider_options = Map::new();
        for namespace in ["openai", self.provider_options_name()] {
            if let Some(values) = options
                .provider_options
                .as_ref()
                .and_then(|all| all.get(namespace))
            {
                for (key, value) in values {
                    let valid = match key.as_str() {
                        "echo" => value.is_boolean(),
                        "user" | "suffix" => value.is_string(),
                        "logitBias" => value
                            .as_object()
                            .is_some_and(|map| map.values().all(Value::is_number)),
                        "logprobs" => value.is_boolean() || value.is_number(),
                        _ => true,
                    };
                    if !valid {
                        return Err(AiMuxError::invalid_argument(format!(
                            "invalid {namespace} provider option {key}"
                        )));
                    }
                }
                provider_options.extend(values.clone());
            }
        }
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model_id));
        for (source, target) in [
            ("echo", "echo"),
            ("logitBias", "logit_bias"),
            ("suffix", "suffix"),
            ("user", "user"),
        ] {
            if let Some(value) = provider_options.get(source) {
                body.insert(target.into(), value.clone());
            }
        }
        match provider_options.get("logprobs") {
            Some(Value::Bool(true)) => {
                body.insert("logprobs".into(), json!(0));
            }
            Some(value) if value.is_number() => {
                body.insert("logprobs".into(), value.clone());
            }
            _ => {}
        }
        for (key, value) in [
            ("max_tokens", json!(options.max_output_tokens)),
            ("temperature", json!(options.temperature)),
            ("top_p", json!(options.top_p)),
            ("frequency_penalty", json!(options.frequency_penalty)),
            ("presence_penalty", json!(options.presence_penalty)),
            ("seed", json!(options.seed)),
        ] {
            if !value.is_null() {
                body.insert(key.into(), value);
            }
        }
        body.insert("prompt".into(), json!(completion_prompt(options)?));
        let mut stop = vec!["\nuser:".to_string()];
        stop.extend(options.stop_sequences.iter().flatten().cloned());
        body.insert("stop".into(), json!(stop));
        Ok((Value::Object(body), warnings))
    }

    async fn request(&self, options: &CallOptions) -> Result<HttpRequest, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        Ok(self
            .config
            .http_request(self.config.url("/completions")?, headers, options))
    }

    fn transform_body(&self, body: Value) -> Value {
        match &self.config.transform_request_body {
            Some(transform) => transform(body),
            None => body,
        }
    }
}

/// The prompt of a text completion (`convertToOpenAICompletionPrompt`).
fn completion_prompt(options: &CallOptions) -> Result<String, AiMuxError> {
    let mut text = String::new();
    for (index, message) in options.prompt.iter().enumerate() {
        match message {
            LanguageModelMessage::System { content, .. } if index == 0 => {
                text.push_str(content);
                text.push_str("\n\n");
            }
            LanguageModelMessage::System { .. } => {
                return Err(AiMuxError::invalid_prompt(
                    "Unexpected system message in prompt",
                ));
            }
            LanguageModelMessage::User { content, .. } => {
                text.push_str("user:\n");
                for part in content {
                    if let UserPart::Text(part) = part {
                        text.push_str(&part.text);
                    }
                }
                text.push_str("\n\n");
            }
            LanguageModelMessage::Assistant { content, .. } => {
                text.push_str("assistant:\n");
                for part in content {
                    match part {
                        AssistantPart::Text(part) => text.push_str(&part.text),
                        AssistantPart::ToolCall(_) => {
                            return Err(AiMuxError::UnsupportedFunctionality(
                                "tool-call messages".into(),
                            ));
                        }
                        _ => {}
                    }
                }
                text.push_str("\n\n");
            }
            LanguageModelMessage::Tool { .. } => {
                return Err(AiMuxError::UnsupportedFunctionality("tool messages".into()));
            }
        }
    }
    text.push_str("assistant:\n");
    Ok(text)
}

/// `convertOpenAICompletionUsage`: only the three counts the response schema
/// keeps; a missing count leaves its total unset.
fn usage(raw: Option<Map<String, Value>>) -> Usage {
    let Some(mut raw) = raw else {
        return Usage::default();
    };
    raw.retain(|key, _| {
        matches!(
            key.as_str(),
            "prompt_tokens" | "completion_tokens" | "total_tokens"
        )
    });
    let input = raw
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    let output = raw
        .get("completion_tokens")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    Usage {
        input_tokens: InputTokenUsage {
            total: input,
            no_cache: Some(input.unwrap_or(0)),
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: output,
            text: Some(output.unwrap_or(0)),
            ..Default::default()
        },
        raw: Some(raw),
    }
}

/// `getResponseMetadata`.
fn metadata(response: &CompletionResponse) -> ResponseMetadata {
    aimux_provider_utils::create_language_model_response_metadata(
        response.id.clone(),
        response.model.clone(),
        response.created.map(|secs| secs as f64),
    )
}

/// `providerMetadata.openai`: the logprobs of the choice, when it has any.
fn logprobs_metadata(logprobs: Option<Value>) -> SharedProviderMetadata {
    provider_namespace(
        "openai",
        match logprobs {
            Some(logprobs) => json!({ "logprobs": logprobs }),
            None => json!({}),
        },
    )
    .expect("provider metadata must be an object")
}

/// `openaiCompletionResponseSchema` / `openaiCompletionChunkSchema`.
#[derive(Deserialize)]
struct CompletionResponse {
    id: Option<String>,
    created: Option<i64>,
    model: Option<String>,
    choices: Vec<CompletionChoice>,
    usage: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    text: String,
    finish_reason: Option<String>,
    logprobs: Option<Value>,
}

fn parse_response(value: Value, streaming: bool) -> Result<CompletionResponse, AiMuxError> {
    let invalid = || AiMuxError::invalid_response_data("invalid completion response");
    if let Some(usage) = value.get("usage").filter(|value| !value.is_null())
        && !["prompt_tokens", "completion_tokens", "total_tokens"]
            .iter()
            .all(|key| usage.get(key).is_some_and(Value::is_number))
    {
        return Err(invalid());
    }
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    for choice in choices {
        let valid_finish = if streaming {
            choice.get("index").is_some_and(Value::is_number)
        } else {
            choice.get("finish_reason").is_some_and(Value::is_string)
        };
        if !valid_finish {
            return Err(invalid());
        }
        if let Some(logprobs) = choice.get("logprobs").filter(|value| !value.is_null()) {
            if !logprobs
                .get("tokens")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().all(Value::is_string))
                || !logprobs
                    .get("token_logprobs")
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.iter().all(Value::is_number))
            {
                return Err(invalid());
            }
            if let Some(top) = logprobs
                .get("top_logprobs")
                .filter(|value| !value.is_null())
                && !top.as_array().is_some_and(|values| {
                    values.iter().all(|value| {
                        value
                            .as_object()
                            .is_some_and(|object| object.values().all(Value::is_number))
                    })
                })
            {
                return Err(invalid());
            }
        }
    }
    Ok(serde_json::from_value(value)?)
}

#[async_trait]
impl LanguageModel for OpenAICompletionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    async fn supported_urls(&self) -> SupportedUrls {
        // No URLs are supported for completion models.
        SupportedUrls::default()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let (body, warnings) = self.arguments(options)?;
        let body = self.transform_body(body);
        let response = aimux_provider_utils::post_json_to_api(
            self.request(options).await?,
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            openai_failed_response_handler(),
        )
        .await?;
        let mut parsed = parse_response(response.value, false)?;
        let metadata = metadata(&parsed);
        let choice = parsed
            .choices
            .drain(..)
            .next()
            .ok_or_else(|| AiMuxError::invalid_response_data("no choices in response"))?;
        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FINISH_OTHER);
        Ok(GenerateResult {
            content: vec![GenerateContent::Text {
                text: choice.text,
                provider_metadata: None,
            }],
            finish_reason,
            usage: usage(parsed.usage),
            warnings,
            provider_metadata: Some(logprobs_metadata(choice.logprobs)),
            request: Some(RequestInfo { body: Some(body) }),
            response: Some(ResponseInfo {
                id: metadata.id,
                timestamp: metadata.timestamp,
                model_id: metadata.model_id,
                headers: Some(response.response_headers),
                body: response.raw_value,
            }),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let (mut body, warnings) = self.arguments(options)?;
        body["stream"] = json!(true);
        body["stream_options"] = json!({ "include_usage": true });
        let body = self.transform_body(body);
        let request = self.request(options).await?;
        let error_url = request.url.clone();
        let response = aimux_provider_utils::post_json_to_api(
            request,
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            openai_failed_response_handler(),
        )
        .await?;
        let mut events = response.value;
        let response_headers = response.response_headers;
        // `throwIfOpenAIStreamErrorBeforeOutput`: an error frame before the
        // first text fails the call itself.
        let mut buffered = Vec::new();
        while let Some(event) = events.next().await {
            if let Ok(parsed_event) = &event {
                if let Some(error) = parsed_event.get("error") {
                    return Err(openai_stream_error(
                        error,
                        &error_url,
                        body.clone(),
                        response_headers.clone(),
                    ));
                }
                let output = parsed_event
                    .get("choices")
                    .and_then(Value::as_array)
                    .is_some_and(|choices| {
                        choices.iter().any(|choice| {
                            choice
                                .get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.is_empty())
                        })
                    });
                buffered.push(event);
                if output {
                    break;
                }
            } else {
                buffered.push(event);
                break;
            }
        }
        let stream = pipe_through(
            futures::stream::iter(buffered).chain(events),
            OpenAICompletionStream {
                ctx: StreamContext::new(options, warnings, error_url, &body, &response_headers),
                ..OpenAICompletionStream::default()
            },
        );
        Ok(crate::stream::strict_guard(
            StreamResult {
                stream: Box::pin(stream),
                request: Some(RequestInfo { body: Some(body) }),
                response: Some(StreamResponseInfo {
                    headers: Some(response_headers),
                }),
            },
            options,
        ))
    }
}

/// The `TransformStream` of `OpenAICompletionLanguageModel.doStream`.
#[derive(Default)]
struct OpenAICompletionStream {
    ctx: StreamContext,
    started: bool,
    /// `other` until a chunk says otherwise.
    finish_reason: Option<FinishReason>,
    usage: Option<Map<String, Value>>,
    logprobs: Option<Value>,
}

impl OpenAICompletionStream {
    fn report_error(&mut self, error: AiMuxError, controller: &mut Controller) {
        self.finish_reason = Some(FINISH_ERROR);
        controller.enqueue(StreamPart::Error { error });
    }
}

impl Transformer for OpenAICompletionStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let event = match event {
            Ok(event) => event,
            Err(error) => return self.report_error(error, controller),
        };
        self.ctx.raw(&event, controller);
        if let Some(error) = event.get("error") {
            // The frame maps to a status (openai-completion-language-model.ts:305-313).
            let error = openai_stream_error(
                error,
                &self.ctx.url,
                self.ctx.request_body.clone(),
                self.ctx.response_headers.clone(),
            );
            return self.report_error(error, controller);
        }
        let parsed: CompletionResponse = match parse_response(event, true) {
            Ok(parsed) => parsed,
            Err(error) => return self.report_error(error, controller),
        };
        if !self.started {
            self.started = true;
            controller.enqueue(StreamPart::ResponseMetadata(metadata(&parsed)));
            controller.text_start("0");
        }
        if parsed.usage.is_some() {
            self.usage = parsed.usage;
        }
        if let Some(choice) = parsed.choices.into_iter().next() {
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(parse_finish_reason(&reason));
            }
            if choice.logprobs.is_some() {
                self.logprobs = choice.logprobs;
            }
            if !choice.text.is_empty() {
                controller.text_delta("0", choice.text);
            }
        }
    }

    fn flush(self, controller: &mut Controller) {
        if self.ctx.fail_without_terminal(
            self.finish_reason.is_some(),
            "a finish_reason",
            controller,
        ) {
            return;
        }
        if self.started {
            controller.text_end("0");
        }
        controller.enqueue(StreamPart::Finish {
            finish_reason: self.finish_reason.unwrap_or(FINISH_OTHER),
            usage: usage(self.usage),
            provider_metadata: Some(logprobs_metadata(self.logprobs)),
        });
    }
}
