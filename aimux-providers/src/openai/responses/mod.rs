//! OpenAI Responses API language model.
//!
//! Implements the [`LanguageModel`] trait against the `/v1/responses`
//! endpoint. The Responses API uses a different request/response format from
//! chat completions: requests carry an `input` array (not `messages`), and
//! streaming events are typed as `response.output_text.delta`,
//! `response.function_call_arguments.delta`, etc.
//!
//! Mirrors the TS `OpenAIResponsesLanguageModel`. The core paths implemented
//! here are:
//! - **Request building**: input array, instructions, store,
//!   previous_response_id, reasoning (effort/summary), response_format
//!   (json_schema/json_object) — see [`convert::build_responses_request_body`].
//! - **Non-streaming**: text, function-call, and reasoning output items -- see
//!   [`OpenAIResponsesModel::do_generate`].
//! - **Streaming**: the `response.created -> output_item.added ->
//!   output_text.delta -> output_text.done -> output_item.done ->
//!   response.completed` main path, plus `function_call_arguments.delta/done`
//!   and `reasoning_summary_text.delta` -- see [`OpenAIResponsesModel::do_stream`].
//!
//! The non-streaming output parser and the streaming SSE event reducer are
//! shared with the Azure OpenAI Responses provider via
//! [`responses_convert`] (RFC-0012 §3.5).

pub mod convert;
pub mod responses_convert;
pub mod types;

pub use convert::{
    ResponsesInputResult, ResponsesRequestBodyResult, build_responses_request_body,
    convert_responses_usage, convert_to_responses_input, map_responses_finish_reason,
    prepare_responses_tools,
};

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send_stream_timed, send_timed};
use aimux_stream::SseStream;

use super::OpenAIConfig;
use responses_convert::build_header_list;

/// An OpenAI Responses API language model.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use
/// the process-wide shared `Client` internally (RFC-0009 §4.1).
///
/// Created via [`OpenAIResponsesProvider`](super::OpenAIResponsesProvider) or
/// directly with [`OpenAIResponsesModel::new`].
pub struct OpenAIResponsesModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAIResponsesModel {
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    pub(crate) fn build_headers(
        &self,
        extra: Option<&HashMap<String, String>>,
    ) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.base_url)
    }

    /// The provider-metadata key: `"azure"` when the provider string contains
    /// `"azure"`, otherwise `"openai"`. Mirrors the TS `providerOptionsName`.
    fn provider_options_name(&self) -> &str {
        if self.config.provider.contains("azure") {
            "azure"
        } else {
            "openai"
        }
    }
}

#[async_trait]
impl LanguageModel for OpenAIResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, false);
        let body = request_result.body;
        let provider_key = self.provider_options_name().to_string();

        let resp = send_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let data: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        responses_convert::build_responses_generate_result(
            &data,
            request_result.warnings,
            provider_key,
            body,
            response_headers,
        )
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let request_result = build_responses_request_body(&self.model_id, options, true);
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_key = self.provider_options_name().to_string();

        // The `store` request option (None by default). Used to decide when
        // reasoning summary parts are concluded.
        let store_flag = options
            .provider_options
            .as_ref()
            .and_then(|m| m.get("openai"))
            .and_then(|o| o.get("store"))
            .and_then(|v| v.as_bool())
            == Some(true);

        let resp = send_stream_timed(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),

                abort_signal: options.abort_signal.clone(),
                call_id: options.call_id.clone(),
            },
            self.config.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
            options.timeout.map(Into::into),
        )
        .await?;

        let response_headers = resp.headers;

        let mut sse_stream = SseStream::new(resp.body);
        let first_event = sse_stream.next().await;
        let stream = responses_convert::build_responses_event_stream(
            first_event,
            sse_stream,
            provider_key,
            warnings,
            store_flag,
        )?;

        Ok(StreamResult {
            stream,
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}
