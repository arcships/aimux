//! Text-completion endpoint shared by the native and compatible packages.

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use aimux_core::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage, UserPart};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::StreamResult;
use aimux_core::result::{GenerateContent, GenerateResult};
use aimux_core::shared::{RequestInfo, ResponseInfo, StreamResponseInfo, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, InputTokenUsage, OutputTokenUsage, ResponseMetadata, Usage,
    Warning,
};
use aimux_provider_utils::HttpRequest;

use super::config::CompatModelConfig;
use super::convert::{parse_finish_reason, to_camel_case};
use crate::openai::config::OpenAIModelConfig;

#[derive(Clone)]
enum CompletionConfig {
    Compatible(CompatModelConfig),
    Native(OpenAIModelConfig),
}

/// A language model for the `/completions` endpoint.
#[derive(Clone)]
pub struct OpenAICompatibleCompletionModel {
    model_id: String,
    config: CompletionConfig,
}

impl OpenAICompatibleCompletionModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self {
            model_id,
            config: CompletionConfig::Compatible(config),
        }
    }

    pub(crate) fn from_native_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self {
            model_id,
            config: CompletionConfig::Native(config),
        }
    }

    fn native(&self) -> bool {
        matches!(self.config, CompletionConfig::Native(_))
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
        let name = self.provider().split('.').next().unwrap_or_default().trim();
        let camel = to_camel_case(name);
        let namespaces = if self.native() {
            vec!["openai", name]
        } else {
            vec![name, camel.as_str()]
        };
        let mut provider_options = Map::new();
        if !self.native()
            && camel != name
            && options
                .provider_options
                .as_ref()
                .is_some_and(|all| all.contains_key(name))
        {
            warnings.push(Warning::Deprecated {
                setting: format!("providerOptions key '{name}'"),
                message: format!("Use '{camel}' instead."),
            });
        }
        for namespace in namespaces {
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
                        "logprobs" if self.native() => value.is_boolean() || value.is_number(),
                        _ => true,
                    };
                    if !valid {
                        return Err(AiMuxError::InvalidArgument(format!(
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
        if self.native() {
            match provider_options.get("logprobs") {
                Some(Value::Bool(true)) => {
                    body.insert("logprobs".into(), json!(0));
                }
                Some(value) if value.is_number() => {
                    body.insert("logprobs".into(), value.clone());
                }
                _ => {}
            }
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
        if !self.native() {
            body.extend(provider_options);
        }
        body.insert("prompt".into(), json!(completion_prompt(options)?));
        let mut stop = vec!["\nuser:".to_string()];
        stop.extend(options.stop_sequences.iter().flatten().cloned());
        body.insert("stop".into(), json!(stop));
        Ok((Value::Object(body), warnings))
    }

    async fn request(&self, options: &CallOptions) -> Result<HttpRequest, AiMuxError> {
        match &self.config {
            CompletionConfig::Compatible(config) => {
                let headers = config.request_headers(options.headers.as_ref()).await?;
                config.http_request("/completions", headers, options)
            }
            CompletionConfig::Native(config) => {
                let headers = config.request_headers(options.headers.as_ref()).await?;
                Ok(config.http_request(config.url("/completions")?, headers, options))
            }
        }
    }

    fn transform_body(&self, body: Value) -> Value {
        match &self.config {
            CompletionConfig::Compatible(_) => body,
            CompletionConfig::Native(config) => match &config.transform_request_body {
                Some(transform) => transform(body),
                None => body,
            },
        }
    }

    fn failed_response_handler(&self) -> aimux_provider_utils::ResponseHandler<AiMuxError> {
        match &self.config {
            CompletionConfig::Compatible(config) => config.failed_response_handler(),
            CompletionConfig::Native(_) => crate::openai::openai_failed_response_handler(),
        }
    }
}

fn completion_prompt(options: &CallOptions) -> Result<String, AiMuxError> {
    let mut text = String::new();
    for (index, message) in options.prompt.iter().enumerate() {
        match message {
            LanguageModelMessage::System { content, .. } if index == 0 => {
                text.push_str(content);
                text.push_str("\n\n");
            }
            LanguageModelMessage::System { .. } => {
                return Err(AiMuxError::InvalidPrompt(
                    "Unexpected system message in prompt".into(),
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

fn parse_response(
    value: Value,
    native: bool,
    streaming: bool,
) -> Result<CompletionResponse, AiMuxError> {
    let invalid = || AiMuxError::InvalidResponseData("invalid completion response".into());
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
        if native && let Some(logprobs) = choice.get("logprobs").filter(|value| !value.is_null()) {
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

fn metadata(response: &CompletionResponse) -> ResponseMetadata {
    ResponseMetadata {
        id: response.id.clone(),
        timestamp: response
            .created
            .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
            .map(|dt| dt.to_rfc3339()),
        model_id: response.model.clone(),
    }
}

fn usage(raw: Option<Map<String, Value>>, native: bool) -> Usage {
    let Some(mut raw) = raw else {
        return Usage::default();
    };
    if native {
        raw.retain(|key, _| {
            matches!(
                key.as_str(),
                "prompt_tokens" | "completion_tokens" | "total_tokens"
            )
        });
    }
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
            total: if native {
                input
            } else {
                Some(input.unwrap_or(0))
            },
            no_cache: Some(input.unwrap_or(0)),
            ..Default::default()
        },
        output_tokens: OutputTokenUsage {
            total: if native {
                output
            } else {
                Some(output.unwrap_or(0))
            },
            text: Some(output.unwrap_or(0)),
            ..Default::default()
        },
        raw: Some(raw),
    }
}

#[async_trait]
impl LanguageModel for OpenAICompatibleCompletionModel {
    fn provider(&self) -> &str {
        match &self.config {
            CompletionConfig::Compatible(config) => &config.provider,
            CompletionConfig::Native(config) => &config.provider,
        }
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn supported_urls(&self) -> SupportedUrls {
        match &self.config {
            CompletionConfig::Compatible(_) => SupportedUrls::default(),
            CompletionConfig::Native(_) => SupportedUrls::default(),
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let (body, warnings) = self.arguments(options)?;
        let body = self.transform_body(body);
        let response = aimux_provider_utils::post_json_to_api(
            self.request(options).await?,
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            self.failed_response_handler(),
        )
        .await?;
        let mut parsed = parse_response(response.value, self.native(), false)?;
        let metadata = metadata(&parsed);
        let choice = parsed
            .choices
            .drain(..)
            .next()
            .ok_or_else(|| AiMuxError::InvalidResponseData("no choices in response".into()))?;
        let provider_metadata = self.native().then(|| {
            provider_namespace(
                "openai",
                match choice.logprobs {
                    Some(logprobs) => json!({ "logprobs": logprobs }),
                    None => json!({}),
                },
            )
            .expect("provider metadata must be an object")
        });
        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            });
        Ok(GenerateResult {
            content: if self.native() || !choice.text.is_empty() {
                vec![GenerateContent::Text {
                    text: choice.text,
                    provider_metadata: None,
                }]
            } else {
                vec![]
            },
            finish_reason,
            usage: usage(parsed.usage, self.native()),
            warnings,
            provider_metadata,
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
        let include_usage = match &self.config {
            CompletionConfig::Compatible(config) => config.chat.include_usage,
            CompletionConfig::Native(_) => true,
        };
        if include_usage {
            body["stream_options"] = json!({ "include_usage": true });
        }
        let body = self.transform_body(body);
        let request = self.request(options).await?;
        let error_url = request.url.clone();
        let response = aimux_provider_utils::post_json_to_api(
            request,
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            self.failed_response_handler(),
        )
        .await?;
        let mut events = response.value;
        let response_headers = response.response_headers;
        let stream_response_headers = response_headers.clone();
        let native = self.native();
        let raw_chunks = options.include_raw_chunks == Some(true);
        let mut buffered = Vec::new();
        if native {
            while let Some(event) = events.next().await {
                if let Ok(parsed_event) = &event {
                    if let Some(error) = parsed_event.get("error") {
                        return Err(crate::openai::openai_stream_error(
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
        }
        let stream_body = body.clone();
        let stream = async_stream::stream! {
            yield Ok(StreamPart::StreamStart { warnings });
            let mut events = futures::stream::iter(buffered).chain(events);
            let mut started = false;
            let mut finish_reason = FinishReason { unified: FinishReasonUnified::Other, raw: None };
            let mut final_usage = None;
            let mut logprobs = None;
            while let Some(event) = events.next().await {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => { finish_reason = FinishReason { unified: FinishReasonUnified::Error, raw: None }; yield Ok(StreamPart::Error { error }); continue; }
                };
                if raw_chunks { yield Ok(StreamPart::Raw { raw_value: event.clone() }); }
                if let Some(error) = event.get("error") {
                    finish_reason = FinishReason { unified: FinishReasonUnified::Error, raw: None };
                    yield Ok(StreamPart::Error { error: crate::openai::openai_stream_error(error, &error_url, stream_body.clone(), response_headers.clone()) });
                    continue;
                }
                let parsed: CompletionResponse = match parse_response(event, native, true) {
                    Ok(parsed) => parsed,
                    Err(error) => { finish_reason = FinishReason { unified: FinishReasonUnified::Error, raw: None }; yield Ok(StreamPart::Error { error }); continue; }
                };
                if !started {
                    started = true;
                    yield Ok(StreamPart::ResponseMetadata(metadata(&parsed)));
                    yield Ok(StreamPart::TextStart { id: "0".into(), provider_metadata: None });
                }
                if parsed.usage.is_some() { final_usage = parsed.usage; }
                if let Some(choice) = parsed.choices.into_iter().next() {
                    if let Some(reason) = choice.finish_reason { finish_reason = parse_finish_reason(&reason); }
                    if native && choice.logprobs.is_some() { logprobs = choice.logprobs; }
                    if !native || !choice.text.is_empty() { yield Ok(StreamPart::TextDelta { id: "0".into(), delta: choice.text, provider_metadata: None }); }
                }
            }
            if started { yield Ok(StreamPart::TextEnd { id: "0".into(), provider_metadata: None }); }
            yield Ok(StreamPart::Finish { finish_reason, usage: usage(final_usage, native), provider_metadata: native.then(|| provider_namespace("openai", match logprobs { Some(logprobs) => json!({ "logprobs": logprobs }), None => json!({}) }).expect("provider metadata must be an object")) });
        };
        Ok(StreamResult {
            stream: Box::pin(stream),
            request: Some(RequestInfo { body: Some(body) }),
            response: Some(StreamResponseInfo {
                headers: Some(stream_response_headers),
            }),
        })
    }
}
