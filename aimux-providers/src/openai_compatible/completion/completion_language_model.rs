//! The text-completion model (`{name}.completion`,
//! `openai-compatible-completion-language-model.ts`).

use aimux_core::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::shared::{RequestInfo, ResponseInfo, StreamResponseInfo};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, Warning};
use aimux_provider_utils::{HttpRequest, Transformer, pipe_through};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::openai_compatible::CompatModelConfig;
use crate::openai_compatible::chat::parse_finish_reason;
use crate::openai_compatible::utils::to_camel_case;
use crate::stream::{Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext};

use super::convert_completion_usage::usage;
use super::convert_to_completion_prompt::completion_prompt;
use super::get_response_metadata::metadata;

/// A language model for the `/completions` endpoint.
#[derive(Clone)]
pub struct OpenAICompatibleCompletionModel {
    model_id: String,
    pub(in crate::openai_compatible) config: CompatModelConfig,
}

impl OpenAICompatibleCompletionModel {
    pub(crate) fn from_config(model_id: String, config: CompatModelConfig) -> Self {
        Self { model_id, config }
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
        let mut provider_options = Map::new();
        if camel != name
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
        for namespace in [name, camel.as_str()] {
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
        body.extend(provider_options);
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
        self.config.http_request("/completions", headers, options)
    }
}

#[derive(Deserialize)]
pub(super) struct CompletionResponse {
    pub(super) id: Option<String>,
    pub(super) created: Option<i64>,
    pub(super) model: Option<String>,
    choices: Vec<CompletionChoice>,
    usage: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
struct CompletionChoice {
    text: String,
    finish_reason: Option<String>,
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
    }
    Ok(serde_json::from_value(value)?)
}

#[async_trait]
impl LanguageModel for OpenAICompatibleCompletionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    async fn supported_urls(&self) -> SupportedUrls {
        SupportedUrls::default()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let (body, warnings) = self.arguments(options)?;
        let response = aimux_provider_utils::post_json_to_api(
            self.request(options).await?,
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            self.config.failed_response_handler(),
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
            content: if !choice.text.is_empty() {
                vec![GenerateContent::Text {
                    text: choice.text,
                    provider_metadata: None,
                }]
            } else {
                vec![]
            },
            finish_reason,
            usage: usage(parsed.usage),
            warnings,
            provider_metadata: None,
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
        if self.config.chat.include_usage {
            body["stream_options"] = json!({ "include_usage": true });
        }
        let request = self.request(options).await?;
        let error_url = request.url.clone();
        let response = aimux_provider_utils::post_json_to_api(
            request,
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            self.config.failed_response_handler(),
        )
        .await?;
        let events = response.value;
        let response_headers = response.response_headers;
        let stream = pipe_through(
            events,
            OpenAICompatibleCompletionStream {
                ctx: StreamContext::new(options, warnings, error_url, &body, &response_headers),
                provider_options_name: self.config.provider_options_name().to_owned(),
                ..OpenAICompatibleCompletionStream::default()
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

/// The `TransformStream` of `OpenAICompatibleCompletionLanguageModel.doStream`.
#[derive(Default)]
struct OpenAICompatibleCompletionStream {
    ctx: StreamContext,
    /// The provider options name the error frames are reported under.
    provider_options_name: String,
    started: bool,
    /// `other` until a chunk says otherwise.
    finish_reason: Option<FinishReason>,
    usage: Option<Map<String, Value>>,
}

impl OpenAICompatibleCompletionStream {
    fn report_error(&mut self, error: AiMuxError, controller: &mut Controller) {
        self.finish_reason = Some(FINISH_ERROR);
        controller.enqueue(StreamPart::Error { error });
    }
}

impl Transformer for OpenAICompatibleCompletionStream {
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
            // Passed on as it is
            // (openai-compatible-completion-language-model.ts:320-325).
            let error = crate::openai_compatible::chat::stream_error(
                &self.provider_options_name,
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
            controller.text_delta("0", choice.text);
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
            provider_metadata: None,
        });
    }
}
