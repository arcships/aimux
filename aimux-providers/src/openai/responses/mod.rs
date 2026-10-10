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
mod provider_events;
pub mod responses_convert;
pub(crate) mod tool_args;
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
        let mut unused_warnings = Vec::new();
        if let Some(aimux_core::options::ResponseFormat::Json {
            schema: Some(schema),
            ..
        }) = &options.response_format
        {
            super::convert::normalize_json_schema(schema, &mut unused_warnings)?;
        }
        for tool in options.tools.iter().flatten() {
            if let aimux_core::tool::Tool::Function(tool) = tool {
                super::convert::normalize_json_schema(&tool.input_schema, &mut unused_warnings)?;
            }
        }
        let pass_through = options
            .provider_options
            .as_ref()
            .and_then(|v| profile.namespace.find_in(v))
            .and_then(|v| v.get("passThroughUnsupportedFiles"))
            .and_then(Value::as_bool)
            == Some(true);
        let caps = super::convert_common::get_model_capabilities(&self.model_id);
        let provider_options = options
            .provider_options
            .as_ref()
            .and_then(|v| profile.namespace.find_in(v));
        if provider_options
            .and_then(|v| v.get("reasoningEffortUpdate"))
            .is_some_and(|v| {
                !matches!(
                    v.as_str(),
                    Some("none" | "low" | "medium" | "high" | "xhigh" | "max")
                )
            })
        {
            return Err(AiMuxError::InvalidArgument("reasoningEffortUpdate".into()));
        }
        let update_incompatible = provider_options.is_some_and(|v| {
            v.get("reasoningMode") == Some(&serde_json::json!("pro"))
                || v.get("contextManagement").is_some()
                || v.get("truncation") == Some(&serde_json::json!("auto"))
        });
        for message in &options.prompt {
            use aimux_core::language_model_message::{
                AssistantPart, LanguageModelMessage, UserPart,
            };
            use aimux_core::shared::FileData;
            if let LanguageModelMessage::System {
                content,
                provider_options,
            } = message
                && let Some(effort) = provider_options
                    .as_ref()
                    .and_then(|v| profile.namespace.find_in(v))
                    .and_then(|v| v.get("reasoningEffortUpdate"))
                    .and_then(Value::as_str)
            {
                if !matches!(effort, "none" | "low" | "medium" | "high" | "xhigh" | "max") {
                    return Err(AiMuxError::InvalidArgument("reasoningEffortUpdate".into()));
                }
                if !content.is_empty()
                    || !caps.supports_configuration_update
                    || update_incompatible
                    || caps
                        .supported_reasoning_efforts
                        .is_some_and(|values| !values.contains(&effort))
                {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "Message-level reasoningEffortUpdate".into(),
                    ));
                }
            }
            let (files, user) = match message {
                LanguageModelMessage::User { content, .. } => (
                    content
                        .iter()
                        .filter_map(|part| match part {
                            UserPart::File(file) => Some(file),
                            UserPart::Text(_) => None,
                        })
                        .collect::<Vec<_>>(),
                    true,
                ),
                LanguageModelMessage::Assistant { content, .. } => (
                    content
                        .iter()
                        .filter_map(|part| match part {
                            AssistantPart::File(file) => Some(file),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                    false,
                ),
                _ => (Vec::new(), false),
            };
            for file in files {
                match &file.data {
                    FileData::Text { .. } if user => {
                        return Err(AiMuxError::UnsupportedFunctionality(
                            "text file parts".into(),
                        ));
                    }
                    FileData::Reference { reference }
                        if !reference.contains_key(profile.namespace.write_key()) =>
                    {
                        return Err(AiMuxError::InvalidArgument(format!(
                            "No file reference for {}",
                            profile.namespace.write_key()
                        )));
                    }
                    FileData::Data { .. } if user => {
                        aimux_provider_utils::resolve_full_media_type(file)?;
                        let media_type = &file.media_type;
                        if !pass_through
                            && !media_type.starts_with("image/")
                            && media_type != "image"
                            && media_type != "application/pdf"
                        {
                            return Err(AiMuxError::UnsupportedFunctionality(format!(
                                "file part media type {media_type}"
                            )));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut result =
            build_responses_request_body_for(profile.namespace, &self.model_id, options, stream)?;
        convert::apply_file_id_prefixes(&mut result.body, &profile.file_id_prefixes);
        if profile.explicit_message_item_type
            && let Some(input) = result.body["input"].as_array_mut()
        {
            for item in input {
                if item.get("role").is_some() {
                    item["type"] = serde_json::json!("message");
                }
            }
        }
        if result.body["input"].as_array().is_some_and(|input| {
            input.windows(2).any(|pair| {
                pair[0]["type"] == "configuration_update"
                    && pair[1]["type"] == "configuration_update"
            })
        }) {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Adjacent reasoning effort configuration updates".into(),
            ));
        }
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

        responses_convert::build_responses_generate_result_with_tools(
            &data,
            &raw_body,
            request_result.warnings,
            provider_key,
            endpoint,
            body,
            response_headers,
            tool_name_mapping(options),
            prompt_approval_tool_call_ids(options),
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
        let stream = responses_convert::build_responses_event_stream_with_tools(
            first_event,
            sse_stream,
            provider_key,
            warnings,
            store_flag,
            endpoint,
            body.clone(),
            response_headers.clone(),
            options.include_raw_chunks == Some(true),
            tool_name_mapping(options),
            prompt_approval_tool_call_ids(options),
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

fn tool_name_mapping(options: &CallOptions) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for tool in options.tools.iter().flatten() {
        if let aimux_core::tool::Tool::Provider(tool) = tool
            && let Some(name) = tool.id.strip_prefix("openai.")
        {
            names.insert(name.to_owned(), tool.name.clone());
        }
    }
    if let Some(name) = options.tools.iter().flatten().find_map(|tool| match tool {
        aimux_core::tool::Tool::Provider(tool)
            if matches!(
                tool.id.as_str(),
                "openai.web_search" | "openai.web_search_preview"
            ) =>
        {
            Some(tool.name.clone())
        }
        _ => None,
    }) {
        names.insert("web_search".into(), name);
    }
    names
}

fn prompt_approval_tool_call_ids(options: &CallOptions) -> HashMap<String, String> {
    use aimux_core::language_model_message::{AssistantPart, LanguageModelMessage};

    let mut mapping = HashMap::new();
    for message in &options.prompt {
        if let LanguageModelMessage::Assistant { content, .. } = message {
            for part in content {
                if let AssistantPart::ToolCall(call) = part
                    && let Some(id) = call
                        .provider_options
                        .as_ref()
                        .and_then(|options| options.get("openai"))
                        .and_then(|options| options.get("approvalRequestId"))
                        .and_then(Value::as_str)
                {
                    mapping.insert(id.to_owned(), call.tool_call_id.clone());
                }
            }
        }
    }
    mapping
}
