//! Codex language model over the OpenAI Responses API.

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::{LanguageModel, SupportedUrls};
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::types::{FinishReason, FinishReasonUnified, Warning};

use crate::openai::internal::{
    OpenAIModelConfig, OpenAIResponsesModel, ResponsesNamespace,
    build_responses_event_stream_with_raw, build_responses_generate_result,
    build_responses_request_body, openai_failed_response_handler,
};

use super::provider::Channel;

/// A Codex language model over the Responses API.
#[derive(Clone)]
pub struct CodexModel {
    model_id: String,
    channel: Channel,
    config: OpenAIModelConfig,
}

impl CodexModel {
    pub(super) fn from_config(
        model_id: String,
        channel: Channel,
        config: OpenAIModelConfig,
    ) -> Self {
        Self {
            model_id,
            channel,
            config,
        }
    }

    /// The shared Responses model, for the API-key channel.
    fn inner(&self) -> OpenAIResponsesModel {
        OpenAIResponsesModel::from_config(self.model_id.clone(), self.config.clone())
    }

    /// Build the request body; the subscription channel always streams and
    /// never stores (mirrors the official Codex client). That rule is part of
    /// this package's request construction, not a body override.
    fn subscription_body(
        &self,
        options: &CallOptions,
    ) -> Result<(Value, Vec<Warning>), AiMuxError> {
        let mut result = build_responses_request_body(&self.model_id, options, true)?;
        result.body["store"] = Value::Bool(false);
        Ok((self.config.transform_body(result.body), result.warnings))
    }

    /// Map a subscription-channel authentication failure to
    /// [`AiMuxError::TokenExpired`] so the integrator can refresh the access
    /// token and rebuild the provider (RFC-0018 §3.2). The only credential on
    /// the subscription channel is the account token, so any 401 from the
    /// endpoint (read from the `status_code` field) means the token is
    /// invalid/expired.
    fn map_subscription_401(error: AiMuxError) -> AiMuxError {
        match error {
            AiMuxError::ApiCall(d) if d.status_code == Some(401) => {
                AiMuxError::TokenExpired(d.message)
            }
            other => other,
        }
    }

    /// Subscription `do_generate`: the endpoint only accepts streaming, so
    /// request with `stream: true` and assemble the final result from the
    /// terminal `response.completed` / `response.incomplete` event (the
    /// official client always streams; non-streaming requests error). A
    /// `response.failed` event is terminal too: an error when its
    /// `response.error` is not null, otherwise a result finishing as the
    /// stream does.
    async fn subscription_generate(
        &self,
        options: &CallOptions,
    ) -> Result<GenerateResult, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let (body, warnings) = self.subscription_body(options)?;

        let endpoint = self.config.url("/responses")?;
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            openai_failed_response_handler(),
        )
        .await
        .map_err(Self::map_subscription_401)?;
        let response_headers = resp.response_headers;
        let mut sse = resp.value;
        // (parsed response object, raw event payload) of the terminal event.
        let mut completed: Option<(Value, String)> = None;
        let mut failure: Option<AiMuxError> = None;
        // The terminal event was a `response.failed` with a null error.
        let mut failed_without_error = false;

        while let Some(event) = sse.next().await {
            match event {
                Ok(data) => {
                    // The endpoint emits `data:`-only SSE frames — the event
                    // type lives in the JSON payload, not the SSE event field.
                    let etype = data.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match etype {
                        "response.completed" | "response.incomplete" => {
                            // The completed event nests the full response
                            // object under `response` — the generate-result
                            // parser expects the response object itself.
                            let raw = data.to_string();
                            completed = Some((response_object(data), raw));
                            break;
                        }
                        // A `response.failed` whose `response.error` is
                        // null is not an error (upstream's `!= null`, which
                        // the shared stream reducer follows): its response
                        // object is the terminal one.
                        "response.failed" if data["response"]["error"].is_null() => {
                            let raw = data.to_string();
                            completed = Some((response_object(data), raw));
                            failed_without_error = true;
                            break;
                        }
                        "response.failed" | "error" => {
                            let err_obj = data
                                .get("error")
                                .or_else(|| data.get("response").and_then(|r| r.get("error")));
                            let message = err_obj
                                .and_then(|e| e.get("message"))
                                .and_then(|m| m.as_str())
                                .unwrap_or("subscription response failed");
                            // Provider-declared in-band failure: keep the
                            // observed 2xx envelope status (§2.2). The shared
                            // helper redacts the raw request context.
                            failure = Some(aimux_provider_utils::create_provider_stream_error(
                                aimux_provider_utils::ProviderStreamError {
                                    message: message.to_owned(),
                                    provider_code: err_obj
                                        .and_then(|e| e.get("code").or_else(|| e.get("type")))
                                        .and_then(|c| c.as_str())
                                        .map(std::string::ToString::to_string),
                                    status_code: Some(200),
                                    is_retryable: None,
                                    data: data.clone(),
                                },
                                &data,
                                endpoint.clone(),
                                body.clone(),
                                response_headers.clone(),
                            ));
                            break;
                        }
                        _ => {}
                    }
                }
                Err(e) if e.is_recoverable_stream_error() => {
                    // A malformed event does not invalidate later SSE frames.
                    // Retain the first parse error in case the stream ends
                    // without a terminal response object.
                    failure.get_or_insert(e);
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }

        match completed {
            Some((data, raw)) => {
                let has_incomplete_reason = data["incomplete_details"]["reason"].as_str().is_some();
                let mut result = build_responses_generate_result(
                    data,
                    raw.as_bytes(),
                    warnings,
                    ResponsesNamespace::OPENAI.write_key().to_string(),
                    endpoint,
                    body,
                    response_headers,
                )?;
                // As the stream reducer's `response.failed` arm: without an
                // `incomplete_details` reason, a failed response finishes
                // with `error`.
                if failed_without_error && !has_incomplete_reason {
                    result.finish_reason = FinishReason {
                        unified: FinishReasonUnified::Error,
                        raw: Some("error".to_string()),
                    };
                }
                Ok(result)
            }
            None => Err(failure.unwrap_or_else(|| {
                AiMuxError::invalid_response_data(
                    "subscription stream ended without response.completed".to_string(),
                )
            })),
        }
    }

    /// Subscription `do_stream`: always streams with `store: false`.
    async fn subscription_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self
            .config
            .request_headers(options.headers.as_ref())
            .await?;
        let (body, warnings) = self.subscription_body(options)?;

        let endpoint = self.config.url("/responses")?;
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            body.clone(),
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            openai_failed_response_handler(),
        )
        .await
        .map_err(Self::map_subscription_401)?;

        let response_headers = resp.response_headers;
        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        let stream = build_responses_event_stream_with_raw(
            first_event,
            sse_stream,
            ResponsesNamespace::OPENAI.write_key().to_string(),
            warnings,
            false,
            endpoint,
            body.clone(),
            response_headers.clone(),
            options.include_raw_chunks == Some(true),
            options.strict_stream == Some(true),
        )?;

        Ok(crate::stream::strict_guard(
            StreamResult {
                stream,
                request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
                response: Some(aimux_core::shared::StreamResponseInfo {
                    headers: Some(response_headers),
                }),
            },
            options,
        ))
    }
}

#[async_trait]
impl LanguageModel for CodexModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    /// HTTP(S) image and PDF URLs, as the OpenAI Responses model declares;
    /// both channels send them as they are.
    async fn supported_urls(&self) -> SupportedUrls {
        self.config.supported_urls.clone()
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        match self.channel {
            Channel::ApiKey => self.inner().do_generate(options).await,
            Channel::ChatGptAccount => self.subscription_generate(options).await,
        }
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        match self.channel {
            Channel::ApiKey => self.inner().do_stream(options).await,
            Channel::ChatGptAccount => self.subscription_stream(options).await,
        }
    }
}

/// The response object of a terminal event: the event nests it under
/// `response`; an event without one is the response object itself.
fn response_object(event: Value) -> Value {
    match event {
        Value::Object(mut event) => match event.remove("response") {
            Some(response) => response,
            None => Value::Object(event),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    // The error a subscription `response.failed` event becomes: the observed
    // 200 status, the event as response body and data.
    #[tokio::test]
    async fn subscription_response_failed_becomes_an_api_call_error() {
        use aimux_core::error::AiMuxError;
        use aimux_core::language_model::LanguageModel;
        use aimux_core::language_model_message::LanguageModelMessage;
        use aimux_core::options::CallOptions;
        use aimux_provider_utils::Resolvable;
        use serde_json::json;

        use crate::test_support::mock_fetch::{Canned, MockFetch};

        let event = json!({
            "type": "response.failed",
            "response": { "error": { "code": "server_error", "message": "Something went wrong" } },
        });
        let mock = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: format!("data: {event}\n\n").into_bytes(),
        }]);
        let provider = super::super::create_codex(super::super::CodexProviderSettings {
            mode: super::super::CodexMode::ChatGptAccount {
                token: Resolvable::Value("token".into()),
                account_id: None,
            },
            fetch: Some(mock.transport()),
            ..Default::default()
        })
        .expect("valid settings");

        let error = provider
            .responses("gpt-5")
            .do_generate(&CallOptions::new(vec![LanguageModelMessage::user_text(
                "Hello",
            )]))
            .await
            .expect_err("the response failed");

        let AiMuxError::ApiCall(error) = error else {
            panic!("expected an ApiCall error, got {error:?}");
        };
        assert_eq!(error.message, "Something went wrong");
        assert_eq!(error.provider_code.as_deref(), Some("server_error"));
        assert_eq!(error.status_code, Some(200));
        assert!(!error.is_retryable);
        assert_eq!(error.data, Some(event.clone()));
        assert_eq!(error.response_body, Some(event.to_string()));
    }

    // A subscription `response.failed` event whose `response.error` is null
    // is not an error, as in the stream: the result finishes with `error`.
    #[tokio::test]
    async fn subscription_response_failed_with_a_null_error_finishes_with_error() {
        use aimux_core::language_model::LanguageModel;
        use aimux_core::language_model_message::LanguageModelMessage;
        use aimux_core::options::CallOptions;
        use aimux_core::types::FinishReasonUnified;
        use aimux_provider_utils::Resolvable;
        use serde_json::json;

        use crate::test_support::mock_fetch::{Canned, MockFetch};

        let event = json!({
            "type": "response.failed",
            "response": {
                "id": "resp_failed_null_error",
                "created_at": 1_741_269_019,
                "model": "gpt-5",
                "output": [],
                "error": null,
                "incomplete_details": null,
            },
        });
        let mock = MockFetch::new(vec![Canned {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            body: format!("data: {event}\n\n").into_bytes(),
        }]);
        let provider = super::super::create_codex(super::super::CodexProviderSettings {
            mode: super::super::CodexMode::ChatGptAccount {
                token: Resolvable::Value("token".into()),
                account_id: None,
            },
            fetch: Some(mock.transport()),
            ..Default::default()
        })
        .expect("valid settings");

        let result = provider
            .responses("gpt-5")
            .do_generate(&CallOptions::new(vec![LanguageModelMessage::user_text(
                "Hello",
            )]))
            .await
            .expect("a null error is not an error");

        assert_eq!(result.finish_reason.unified, FinishReasonUnified::Error);
        assert_eq!(result.finish_reason.raw.as_deref(), Some("error"));
        assert!(result.content.is_empty());
    }
}
