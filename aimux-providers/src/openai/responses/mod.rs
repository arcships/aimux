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

pub(crate) use convert::ResponsesProfile;
pub use convert::{
    ResponsesInputResult, ResponsesNamespace, ResponsesRequestBodyResult,
    build_responses_request_body, build_responses_request_body_for, convert_responses_usage,
    convert_to_responses_input, map_responses_finish_reason, prepare_responses_tools,
};

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};

use super::config::OpenAIModelConfig;

/// An OpenAI Responses API language model.
///
/// Does **not** hold an HTTP client — the `aimux-provider-utils` API helpers use
/// the process-wide shared `Client` internally (RFC-0009 §4.1).
///
/// Created with
/// [`OpenAIProvider::responses`](crate::openai::OpenAIProvider::responses).
pub struct OpenAIResponsesModel {
    model_id: String,
    config: OpenAIModelConfig,
}

impl OpenAIResponsesModel {
    pub(crate) fn from_config(model_id: String, config: OpenAIModelConfig) -> Self {
        Self { model_id, config }
    }

    /// The request headers for one call: provider headers resolved now, with
    /// the per-call headers layered over them.
    pub(crate) async fn request_headers(
        &self,
        call_headers: Option<&HashMap<String, String>>,
    ) -> Result<Vec<(String, String)>, AiMuxError> {
        self.config.request_headers(call_headers).await
    }

    fn endpoint(&self) -> Result<String, AiMuxError> {
        self.config.url("/responses")
    }

    /// The request body for one call: built with the host's providerOptions
    /// namespace, file-id prefixes applied, then the provider-level rewrite.
    fn request_body(
        &self,
        options: &CallOptions,
        stream: bool,
    ) -> Result<ResponsesRequestBodyResult, AiMuxError> {
        let profile = &self.config.responses;
        let mut result =
            build_responses_request_body_for(profile.namespace, &self.model_id, options, stream)?;
        convert::apply_file_id_prefixes(&mut result.body, &profile.file_id_prefixes);
        result.body = self.config.transform_body(result.body);
        Ok(result)
    }

    /// The provider-metadata key of the host.
    fn provider_options_name(&self) -> &str {
        self.config.responses.namespace.write_key()
    }
}

#[async_trait]
impl LanguageModel for OpenAIResponsesModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }

    fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
        self.config.supported_urls.clone()
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.request_headers(options.headers.as_ref()).await?;
        let request_result = self.request_body(options, false)?;
        let body = request_result.body;
        let provider_key = self.provider_options_name().to_string();

        let endpoint = self.endpoint()?;
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            body.clone(),
            aimux_provider_utils::create_json_response_handler::<Value>(),
            super::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp
            .raw_value
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let data = resp.value;

        responses_convert::build_responses_generate_result(
            &data,
            &raw_body,
            request_result.warnings,
            provider_key,
            endpoint,
            body,
            response_headers,
        )
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.request_headers(options.headers.as_ref()).await?;
        let request_result = self.request_body(options, false)?;
        let body = request_result.body;
        let warnings = request_result.warnings;
        let provider_key = self.provider_options_name().to_string();

        // The `store` request option (None by default). Used to decide when
        // reasoning summary parts are concluded.
        let store_flag = options
            .provider_options
            .as_ref()
            .and_then(|m| self.config.responses.namespace.find_in(m))
            .and_then(|o| o.get("store"))
            .and_then(serde_json::Value::as_bool)
            == Some(true);

        let endpoint = self.endpoint()?;
        let mut stream_body = body.clone();
        stream_body["stream"] = Value::Bool(true);
        let resp = aimux_provider_utils::post_json_to_api(
            self.config.http_request(endpoint.clone(), headers, options),
            stream_body,
            aimux_provider_utils::create_event_source_response_handler::<Value>(),
            super::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let first_event = match sse_stream.next().await {
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            first_event => first_event,
        };
        let stream = responses_convert::build_responses_event_stream(
            first_event,
            sse_stream,
            provider_key,
            warnings,
            store_flag,
            endpoint,
            body.clone(),
            response_headers.clone(),
        )?;

        Ok(StreamResult {
            stream,
            request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
            response: Some(aimux_core::shared::StreamResponseInfo {
                headers: Some(response_headers),
            }),
        })
    }
}
