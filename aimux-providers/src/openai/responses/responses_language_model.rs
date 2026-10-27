//! The Responses language model (openai-responses-language-model.ts).
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
//!   (json_schema/json_object) — see [`build_responses_request_body`].
//! - **Non-streaming**: text, function-call, and reasoning output items -- see
//!   [`OpenAIResponsesModel::do_generate`].
//! - **Streaming**: the `response.created -> output_item.added ->
//!   output_text.delta -> output_text.done -> output_item.done ->
//!   response.completed` main path, plus `function_call_arguments.delta/done`
//!   and `reasoning_summary_text.delta` -- see [`OpenAIResponsesModel::do_stream`].
//!
//! Vendors whose Responses implementations speak the OpenAI wire format
//! (currently OpenAI and Azure OpenAI) share the parts that are byte-identical
//! across them (RFC-0012 §3.5): the non-streaming output parsing in
//! [`build_responses_generate_result`] and the streaming SSE event reducer.
//!
//! Vendors with genuinely different protocols (xAI, HuggingFace, the generic
//! `open_responses` provider) keep their own request/streaming logic and reuse
//! only the small shared helpers where they are byte-identical. Per the RFC,
//! genuinely different streaming loops are **not** force-merged into one
//! function — only the shared framework is extracted. The provider tool output
//! items map to content parts in `provider_tool_content`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;

use aimux_core::AiMuxError;
use aimux_core::error::ApiCallError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::{CallOptions, ResponseFormat};
use aimux_core::result::{
    GenerateContent, GenerateResult, RawToolApprovalRequest, ReasoningOutput, Source, StreamResult,
};
use aimux_core::shared::{SharedProviderOptions, provider_namespace};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::{RawToolCall, Tool, ToolResult};
use aimux_core::types::{
    FinishReason, FinishReasonUnified, NamespacedKind, ProviderMetadata, ReasoningEffort, Warning,
};
use aimux_provider_utils::{Transformer, pipe_through};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{Value, json};

use crate::openai::config::OpenAIModelConfig;
use crate::openai::language_model_capabilities::{
    ModelCapabilities, SystemMessageMode, get_model_capabilities,
};
use crate::openai::responses::convert_responses_usage::{convert_responses_usage, parse_usage};
use crate::openai::responses::convert_to_responses_input::{
    apply_file_id_prefixes, convert_responses_input_with_conversation,
};
use crate::openai::responses::map_responses_finish_reason::map_responses_finish_reason;
use crate::openai::responses::responses_api::ResponsesUsage;
use crate::openai::responses::responses_language_model_options::{
    ResponsesNamespace, openai_option,
};
use crate::openai::responses::responses_prepare_tools::prepare_responses_tools_for;
use crate::stream::{Controller, Emit, FINISH_ERROR, FINISH_OTHER, StreamContext};

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
            crate::openai::normalize_json_schema::normalize_json_schema(
                schema,
                &mut unused_warnings,
            )?;
        }
        for tool in options.tools.iter().flatten() {
            if let aimux_core::tool::Tool::Function(tool) = tool {
                crate::openai::normalize_json_schema::normalize_json_schema(
                    &tool.input_schema,
                    &mut unused_warnings,
                )?;
            }
        }
        let pass_through = options
            .provider_options
            .as_ref()
            .and_then(|v| profile.namespace.find_in(v))
            .and_then(|v| v.get("passThroughUnsupportedFiles"))
            .and_then(Value::as_bool)
            == Some(true);
        let caps =
            crate::openai::language_model_capabilities::get_model_capabilities(&self.model_id);
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
            return Err(AiMuxError::invalid_argument("reasoningEffortUpdate"));
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
                    return Err(AiMuxError::invalid_argument("reasoningEffortUpdate"));
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
                    FileData::Reference { reference } => {
                        aimux_provider_utils::resolve_provider_reference(
                            reference,
                            profile.namespace.write_key(),
                        )?;
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
        apply_file_id_prefixes(&mut result.body, &profile.file_id_prefixes);
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

    async fn supported_urls(&self) -> aimux_core::language_model::SupportedUrls {
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
            crate::openai::error::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;
        let raw_body = resp
            .raw_value
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        let data = resp.value;

        build_responses_generate_result_with_tools(
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
            crate::openai::error::openai_failed_response_handler(),
        )
        .await?;

        let response_headers = resp.response_headers;

        let mut sse_stream = resp.value;
        let events_before_output =
            read_until_output(&mut sse_stream, &endpoint, &body, &response_headers).await?;
        let stream = build_responses_event_stream_with_tools(
            None,
            futures::stream::iter(events_before_output).chain(sse_stream),
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

/// The provider tools whose names the AI SDK maps back to the caller's tool
/// names (`providerToolNames` in openai-responses-language-model.ts).
const PROVIDER_TOOL_NAMES: &[&str] = &[
    "code_interpreter",
    "computer",
    "file_search",
    "image_generation",
    "local_shell",
    "shell",
    "web_search",
    "web_search_preview",
    "mcp",
    "apply_patch",
    "tool_search",
    "programmatic_tool_calling",
];

fn tool_name_mapping(options: &CallOptions) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for tool in options.tools.iter().flatten() {
        if let aimux_core::tool::Tool::Provider(tool) = tool
            && let Some(name) = tool.id.as_str().strip_prefix("openai.")
            && PROVIDER_TOOL_NAMES.contains(&name)
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

// -- Request body ------------------------------------------------------------

/// Result of building a Responses request body.
pub struct ResponsesRequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Compatibility warnings for call options the Responses API does not carry.
fn push_unsupported_call_option_warnings(options: &CallOptions, warnings: &mut Vec<Warning>) {
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
    if options.presence_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "presencePenalty".to_string(),
            details: None,
        });
    }
    if options.frequency_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "frequencyPenalty".to_string(),
            details: None,
        });
    }
    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".to_string(),
            details: None,
        });
    }
}

/// Resolve the Responses reasoning config: `reasoningEffort` (provider option
/// wins over top-level `reasoning`), `reasoningSummary` (defaults to "detailed"
/// when an effort other than "none" applies), and whether the model reasons.
fn resolve_responses_reasoning(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    options: &CallOptions,
    caps: &ModelCapabilities,
) -> (Option<String>, Option<String>, bool) {
    let resolved_reasoning_effort: Option<String> =
        openai_option(ns, provider_opts, "reasoningEffort")
            .map(|v| {
                v.as_str()
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .or_else(|| {
                if options.reasoning.is_some_and(ReasoningEffort::is_custom) {
                    options.reasoning.map(|r| r.to_string())
                } else {
                    None
                }
            });

    let resolved_reasoning_summary = match openai_option(ns, provider_opts, "reasoningSummary") {
        Some(value) => value.as_str().map(str::to_owned),
        None => resolved_reasoning_effort
            .as_deref()
            .filter(|effort| *effort != "none")
            .map(|_| "detailed".to_string()),
    };

    let is_reasoning_model = openai_option(ns, provider_opts, "forceReasoning")
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(caps.is_reasoning_model);

    (
        resolved_reasoning_effort,
        resolved_reasoning_summary,
        is_reasoning_model,
    )
}

/// Warn when `conversation` and `previousResponseId` are both set.
fn warn_conversation_conflict(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    warnings: &mut Vec<Warning>,
) {
    let has_conversation = openai_option(ns, provider_opts, "conversation").is_some();
    let has_previous_response_id = openai_option(ns, provider_opts, "previousResponseId").is_some();
    if has_conversation && has_previous_response_id {
        warnings.push(Warning::Unsupported {
            feature: "conversation".to_string(),
            details: Some(
                "conversation and previousResponseId cannot be used together".to_string(),
            ),
        });
    }
}

fn resolve_responses_system_message_mode(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    caps: &ModelCapabilities,
) -> SystemMessageMode {
    openai_option(ns, provider_opts, "systemMessageMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
        .map(|s| match s.as_str() {
            "developer" => SystemMessageMode::Developer,
            "remove" => SystemMessageMode::Remove,
            _ => SystemMessageMode::System,
        })
        .unwrap_or(if is_reasoning_model {
            SystemMessageMode::Developer
        } else {
            caps.system_message_mode
        })
}

/// temperature / top_p, subject to reasoning-model restrictions.
fn apply_responses_sampling(
    ns: ResponsesNamespace,
    body: &mut Value,
    options: &CallOptions,
    caps: &ModelCapabilities,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    warnings: &mut Vec<Warning>,
) {
    let provider_opts = &options.provider_options;
    let mut temperature = options.temperature;
    let mut top_p = options.top_p;

    if is_reasoning_model {
        let allow_non_reasoning = resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters;
        if !allow_non_reasoning {
            if temperature.is_some() {
                temperature = None;
                warnings.push(Warning::Unsupported {
                    feature: "temperature".to_string(),
                    details: Some("temperature is not supported for reasoning models".to_string()),
                });
            }
            if top_p.is_some() {
                top_p = None;
                warnings.push(Warning::Unsupported {
                    feature: "topP".to_string(),
                    details: Some("topP is not supported for reasoning models".to_string()),
                });
            }
        }
    } else {
        for key in [
            "reasoningEffort",
            "reasoningSummary",
            "reasoningMode",
            "reasoningContext",
        ] {
            if openai_option(ns, provider_opts, key).is_some() {
                warnings.push(Warning::Unsupported {
                    feature: key.to_string(),
                    details: Some(format!("{key} is not supported for non-reasoning models")),
                });
            }
        }
    }

    if let Some(t) = temperature {
        body["temperature"] = json!(t);
    }
    if let Some(p) = top_p {
        body["top_p"] = json!(p);
    }
}

/// `text.format` (json_schema / json_object) plus `verbosity`.
fn apply_responses_text_format(
    ns: ResponsesNamespace,
    body: &mut Value,
    options: &CallOptions,
    provider_opts: &Option<SharedProviderOptions>,
    warnings: &mut Vec<Warning>,
) {
    if let Some(ref rf) = options.response_format {
        match rf {
            ResponseFormat::Text => {}
            ResponseFormat::Json {
                schema,
                name,
                description,
            } => {
                let mut text = json!({});
                match schema {
                    Some(schema) => {
                        let strict_json = openai_option(ns, provider_opts, "strictJsonSchema")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        text["format"] = json!({
                            "type": "json_schema",
                            "strict": strict_json,
                            "name": name.clone().unwrap_or_else(|| "response".to_string()),
                            "schema": crate::openai::normalize_json_schema::normalize_json_schema(schema, warnings).unwrap_or_else(|_| schema.clone()),
                        });
                        if let Some(description) = description {
                            text["format"]["description"] = json!(description);
                        }
                    }
                    None => {
                        text["format"] = json!({ "type": "json_object" });
                    }
                }
                body["text"] = text;
            }
        }
    }

    if let Some(verbosity) = openai_option(ns, provider_opts, "textVerbosity") {
        let text = body.get_mut("text").and_then(|t| t.as_object_mut());
        match text {
            Some(obj) => {
                obj.insert("verbosity".to_string(), verbosity);
            }
            None => {
                body["text"] = json!({ "verbosity": verbosity });
            }
        }
    }
}

/// The computed `include` list (store=false on reasoning models adds
/// `reasoning.encrypted_content`).
fn resolve_responses_include(
    ns: ResponsesNamespace,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    tools: Option<&[Tool]>,
) -> Option<Vec<Value>> {
    let mut include: Option<Vec<Value>> =
        openai_option(ns, provider_opts, "include").and_then(|v| v.as_array().cloned());

    let add_include = |key: &str, inc: &mut Option<Vec<Value>>| {
        let already = inc
            .as_ref()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(key)));
        if !already {
            match inc {
                Some(arr) => arr.push(json!(key)),
                None => *inc = Some(vec![json!(key)]),
            }
        }
    };

    // store defaults to true; only the explicit `false` triggers encrypted_content.
    let store_explicit = openai_option(ns, provider_opts, "store").and_then(|v| v.as_bool());
    if store_explicit == Some(false) && is_reasoning_model {
        add_include("reasoning.encrypted_content", &mut include);
    }

    let top_logprobs = openai_option(ns, provider_opts, "logprobs")
        .and_then(|v| if v == true { Some(20) } else { v.as_u64() });
    if top_logprobs.is_some_and(|value| value > 0) {
        add_include("message.output_text.logprobs", &mut include);
    }
    for tool in tools.into_iter().flatten() {
        if let Tool::Provider(tool) = tool {
            match tool.id.as_str() {
                "openai.web_search" | "openai.web_search_preview"
                    if openai_option(ns, provider_opts, "includeWebSearchSources")
                        != Some(json!(false)) =>
                {
                    add_include("web_search_call.action.sources", &mut include)
                }
                "openai.code_interpreter" => {
                    add_include("code_interpreter_call.outputs", &mut include)
                }
                _ => {}
            }
        }
    }
    include
}

/// Pass-through of the remaining Responses provider options (only sent when
/// set).
fn apply_responses_provider_options(
    ns: ResponsesNamespace,
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
) {
    let mut set = |key: &str, body_key: &str| {
        if let Some(v) = openai_option(ns, provider_opts, key) {
            body[body_key] = v;
        }
    };
    set("conversation", "conversation");
    set("maxToolCalls", "max_tool_calls");
    set("metadata", "metadata");
    set("parallelToolCalls", "parallel_tool_calls");
    set("previousResponseId", "previous_response_id");
    set("user", "user");
    set("instructions", "instructions");
    set("promptCacheKey", "prompt_cache_key");
    set("promptCacheRetention", "prompt_cache_retention");
    set("safetyIdentifier", "safety_identifier");
    set("truncation", "truncation");
    if let Some(Value::Object(options)) = openai_option(ns, provider_opts, "promptCacheOptions") {
        body["prompt_cache_options"] = Value::Object(
            options
                .into_iter()
                .filter(|(key, _)| matches!(key.as_str(), "mode" | "ttl"))
                .collect(),
        );
    }
    if let Some(value) = openai_option(ns, provider_opts, "logprobs").and_then(|v| {
        if v == true {
            Some(json!(20))
        } else {
            v.as_u64().map(|n| json!(n))
        }
    }) {
        body["top_logprobs"] = value;
    }
    if let Some(values) =
        openai_option(ns, provider_opts, "contextManagement").and_then(|v| v.as_array().cloned())
    {
        body["context_management"] = json!(
            values
                .into_iter()
                .map(|v| {
                    let mut entry = json!({ "type": v["type"] });
                    if let Some(threshold) = v.get("compactThreshold") {
                        entry["compact_threshold"] = threshold.clone();
                    }
                    entry
                })
                .collect::<Vec<_>>()
        );
    }
}

/// `service_tier` with model-capability validation.
fn apply_responses_service_tier(
    ns: ResponsesNamespace,
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
    caps: &ModelCapabilities,
    warnings: &mut Vec<Warning>,
) {
    if let Some(st) = openai_option(ns, provider_opts, "serviceTier")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string))
    {
        match st.as_str() {
            "flex" if !caps.supports_flex_processing => {
                warnings.push(Warning::Unsupported {
                    feature: "serviceTier".to_string(),
                    details: Some(
                        "flex processing is only available for o3, o4-mini, and gpt-5 models"
                            .to_string(),
                    ),
                });
            }
            "priority" | "fast" if !caps.supports_priority_processing => {
                warnings.push(Warning::Unsupported {
                    feature: "serviceTier".to_string(),
                    details: Some("priority processing is only available for supported models (gpt-4, gpt-5, gpt-5-mini, o3, o4-mini) and requires Enterprise access. gpt-5-nano is not supported".to_string()),
                });
            }
            _ => {
                body["service_tier"] = json!(st);
            }
        }
    }
}

/// `reasoning` block for reasoning models.
fn apply_responses_reasoning_block(
    ns: ResponsesNamespace,
    body: &mut Value,
    provider_opts: &Option<SharedProviderOptions>,
    is_reasoning_model: bool,
    resolved_reasoning_effort: &Option<String>,
    resolved_reasoning_summary: &Option<String>,
) {
    if !is_reasoning_model {
        return;
    }
    let effort = resolved_reasoning_effort.as_ref();
    let summary = resolved_reasoning_summary.as_ref();
    let mode = openai_option(ns, provider_opts, "reasoningMode")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));
    let context = openai_option(ns, provider_opts, "reasoningContext")
        .and_then(|v| v.as_str().map(std::string::ToString::to_string));

    if effort.is_some() || summary.is_some() || mode.is_some() || context.is_some() {
        let mut reasoning = json!({});
        if let Some(e) = effort {
            reasoning["effort"] = json!(e);
        }
        if let Some(s) = summary {
            reasoning["summary"] = json!(s);
        }
        if let Some(m) = mode {
            reasoning["mode"] = json!(m);
        }
        if let Some(c) = context {
            reasoning["context"] = json!(c);
        }
        body["reasoning"] = reasoning;
    }
}

/// Build the OpenAI Responses API request body and warnings.
///
/// # Errors
/// Rejects unsupported prompt parts and invalid tool arguments.
pub fn build_responses_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<ResponsesRequestBodyResult, AiMuxError> {
    build_responses_request_body_for(ResponsesNamespace::OPENAI, model_id, options, stream)
}

/// [`build_responses_request_body`] for a host with its own providerOptions
/// namespace.
///
/// # Errors
/// Rejects unsupported prompt parts and invalid tool arguments.
pub fn build_responses_request_body_for(
    ns: ResponsesNamespace,
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<ResponsesRequestBodyResult, AiMuxError> {
    let mut warnings: Vec<Warning> = Vec::new();
    let caps = get_model_capabilities(model_id);
    let provider_opts = &options.provider_options;

    // -- Warnings for unsupported call options --
    push_unsupported_call_option_warnings(options, &mut warnings);

    // -- Reasoning resolution --
    let (mut resolved_reasoning_effort, mut resolved_reasoning_summary, is_reasoning_model) =
        resolve_responses_reasoning(ns, provider_opts, options, &caps);
    if let Some(efforts) = caps.supported_reasoning_efforts
        && resolved_reasoning_effort
            .as_deref()
            .is_some_and(|effort| !efforts.contains(&effort))
    {
        warnings.push(Warning::Unsupported {
            feature: "reasoningEffort".to_string(),
            details: Some(format!(
                "{model_id} only supports the following reasoning efforts: {}",
                efforts.join(", ")
            )),
        });
        resolved_reasoning_effort = None;
        resolved_reasoning_summary = openai_option(ns, provider_opts, "reasoningSummary")
            .and_then(|v| v.as_str().map(str::to_owned));
    }

    // -- conversation + previousResponseId conflict --
    warn_conversation_conflict(ns, provider_opts, &mut warnings);

    // -- System message mode --
    let system_message_mode =
        resolve_responses_system_message_mode(ns, provider_opts, is_reasoning_model, &caps);

    // -- Input conversion --
    let store_bool = openai_option(ns, provider_opts, "store")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let has_previous_response_id = openai_option(ns, provider_opts, "previousResponseId").is_some();
    let mut input_result = convert_responses_input_with_conversation(
        ns,
        &options.prompt,
        system_message_mode,
        store_bool,
        has_previous_response_id,
        openai_option(ns, provider_opts, "conversation").is_some(),
        options.tools.as_deref(),
    )?;
    if let Some(effort) = openai_option(ns, provider_opts, "reasoningEffortUpdate")
        .and_then(|v| v.as_str().map(str::to_owned))
    {
        let reason = if !caps.supports_configuration_update {
            Some("reasoningEffortUpdate is only supported by GPT-6 and later models".to_string())
        } else if openai_option(ns, provider_opts, "reasoningMode") == Some(json!("pro"))
            || openai_option(ns, provider_opts, "contextManagement").is_some()
            || openai_option(ns, provider_opts, "truncation") == Some(json!("auto"))
        {
            Some("reasoningEffortUpdate requires standard reasoning mode without automatic compaction or automatic truncation".to_string())
        } else {
            caps.supported_reasoning_efforts
                .filter(|values| !values.contains(&effort.as_str()))
                .map(|values| {
                    format!(
                        "{model_id} only supports the following reasoning efforts: {}",
                        values.join(", ")
                    )
                })
        };
        if let Some(details) = reason {
            warnings.push(Warning::Unsupported {
                feature: "reasoningEffortUpdate".to_string(),
                details: Some(details),
            });
        } else if input_result.input.first().is_none_or(|item| {
            item["type"] != "configuration_update" || item["reasoning"]["effort"] != effort
        }) {
            input_result.input.insert(
                0,
                json!({ "type": "configuration_update", "reasoning": { "effort": effort } }),
            );
        }
    }
    if openai_option(ns, provider_opts, "compactionTrigger") == Some(json!(true)) {
        input_result
            .input
            .push(json!({ "type": "compaction_trigger" }));
    }
    warnings.extend(input_result.warnings);

    // -- Base body --
    let mut body = json!({
        "model": model_id,
        "input": input_result.input,
    });

    if stream {
        body["stream"] = json!(true);
    }

    if let Some(max_tokens) = options.max_output_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    // temperature / top_p (subject to reasoning-model restrictions)
    apply_responses_sampling(
        ns,
        &mut body,
        options,
        &caps,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &mut warnings,
    );

    // -- Response format (text.format) + verbosity --
    apply_responses_text_format(ns, &mut body, options, provider_opts, &mut warnings);

    // -- include (computed) --
    let include = resolve_responses_include(
        ns,
        provider_opts,
        is_reasoning_model,
        options.tools.as_deref(),
    );
    if let Some(inc) = include {
        body["include"] = json!(inc);
    }

    // -- store (only sent when explicitly set) --
    if let Some(s) = openai_option(ns, provider_opts, "store").and_then(|v| v.as_bool()) {
        body["store"] = json!(s);
    }

    // -- Other provider options (only sent when set) --
    apply_responses_provider_options(ns, &mut body, provider_opts);
    if caps.supports_configuration_update && body.get("prompt_cache_retention").is_some() {
        body.as_object_mut()
            .expect("request body is an object")
            .remove("prompt_cache_retention");
        warnings.push(Warning::Unsupported {
            feature: "promptCacheRetention".to_string(),
            details: Some("promptCacheRetention is not supported by GPT-6 and later models; use promptCacheOptions instead".to_string()),
        });
    }

    if is_reasoning_model
        && caps.supported_reasoning_efforts.is_some()
        && !(resolved_reasoning_effort.as_deref() == Some("none")
            && caps.supports_non_reasoning_parameters)
    {
        let mut removed = body
            .as_object_mut()
            .expect("request body is an object")
            .remove("top_logprobs")
            .is_some();
        if let Some(include) = body.get_mut("include").and_then(Value::as_array_mut) {
            let len = include.len();
            include.retain(|value| value.as_str() != Some("message.output_text.logprobs"));
            removed |= include.len() != len;
        }
        if body
            .get("include")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            body.as_object_mut()
                .expect("request body is an object")
                .remove("include");
        }
        if removed {
            warnings.push(Warning::Unsupported {
                feature: "logprobs".to_string(),
                details: Some("logprobs is not supported for reasoning models".to_string()),
            });
        }
    }

    // -- service_tier (with capability validation) --
    apply_responses_service_tier(ns, &mut body, provider_opts, &caps, &mut warnings);

    // -- reasoning block (reasoning models only) --
    apply_responses_reasoning_block(
        ns,
        &mut body,
        provider_opts,
        is_reasoning_model,
        &resolved_reasoning_effort,
        &resolved_reasoning_summary,
    );

    // -- Tools --
    let allowed_tools = openai_option(ns, provider_opts, "allowedTools");
    let prepared = prepare_responses_tools_for(
        &options.tools,
        options.tool_choice.as_ref(),
        allowed_tools.as_ref(),
        caps.supports_configuration_update,
    )?;
    if let Some(tools) = prepared.tools {
        body["tools"] = json!(tools);
        if let Some(tc) = prepared.tool_choice {
            body["tool_choice"] = tc;
        }
    }
    for tw in prepared.tool_warnings {
        warnings.push(tw);
    }

    Ok(ResponsesRequestBodyResult { body, warnings })
}

/// Pinned, boxed stream of model stream parts.
///
/// Matches the `stream` field of [`aimux_core::result::StreamResult`]. Used as
/// the return type of the shared streaming reducer so the boxed trait object
/// does not leak a complex type into call sites.
pub type ResponsesEventStream = Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>;

// -- Non-streaming output parsing --------------------------------------------

/// Parse a non-streaming Responses API JSON body into a [`GenerateResult`].
///
/// Shared verbatim by the OpenAI and Azure providers: both speak the same
/// Responses wire format for non-streaming output (top-level error, `output`
/// array of `message`/`function_call`/`custom_tool_call`/`reasoning` items,
/// `incomplete_details`, `usage`, provider metadata with `responseId` /
/// `reasoningContext` / `serviceTier`). Vendor callers supply the parsed `data`,
/// the observed HTTP `status` and full `raw_body` (evidence for in-band 2xx
/// errors), the request `body`/`response_headers` to attach, and the
/// provider-metadata namespace `provider_key` ("openai" / "azure").
///
/// # Errors
///
/// Returns `ApiCall` for in-band 2xx errors (top-level `error` object, error
/// status, `incomplete_details`) and `InvalidResponseData` when required
/// fields such as `output` are missing.
pub fn build_responses_generate_result(
    data: &Value,
    raw_body: &str,
    request_warnings: Vec<Warning>,
    provider_key: String,
    request_url: String,
    body: Value,
    response_headers: HashMap<String, String>,
) -> Result<GenerateResult, AiMuxError> {
    build_responses_generate_result_with_tools(
        data,
        raw_body,
        request_warnings,
        provider_key,
        request_url,
        body,
        response_headers,
        HashMap::new(),
        HashMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_generate_result_with_tools(
    data: &Value,
    raw_body: &str,
    request_warnings: Vec<Warning>,
    provider_key: String,
    request_url: String,
    body: Value,
    response_headers: HashMap<String, String>,
    tool_names: HashMap<String, String>,
    prompt_approval_tool_call_ids: HashMap<String, String>,
) -> Result<GenerateResult, AiMuxError> {
    // Top-level error field.
    if let Some(err_obj) = data.get("error")
        && err_obj.is_object()
    {
        let message = err_obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Responses API error");
        let provider_code = err_obj
            .get("type")
            .or_else(|| err_obj.get("code"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
            // Provider-declared in-band failure: keep the observed 2xx
            // envelope status and the full raw body (§2.2). The request body
            // is raw here (the exchange-path redaction ran on its own copy),
            // so redact before it lands in the public error.
            status_code: Some(200),
            provider_code,
            response_body: Some(raw_body.to_string()),
            response_headers: Some(response_headers.clone()),
            ..ApiCallError::new(
                message,
                request_url,
                aimux_provider_utils::redact_error_context(body.clone()),
            )
        })));
    }

    let output = data.get("output").and_then(|v| v.as_array());
    let output = output.ok_or_else(|| {
        let detail = data
            .get("incomplete_details")
            .and_then(|d| d.get("reason"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // A success response that cannot yield a usable result (§2.2).
        AiMuxError::invalid_response_data(if detail.is_empty() {
            "Responses API returned no output".to_string()
        } else {
            format!("Responses API returned no output ({detail})")
        })
    })?;

    let shell_provider_executed = hosted_shell(&body);
    let mut content: Vec<GenerateContent> = Vec::new();
    let mut has_function_call = false;
    let mut logprobs = Vec::new();
    let mut hosted_tool_search_ids = VecDeque::new();

    for part in output {
        match part.get("type").and_then(|v| v.as_str()) {
            Some("message") => {
                if let Some(content_parts) = part.get("content").and_then(|v| v.as_array()) {
                    // The AI SDK's response schema lets a message hold only
                    // `output_text` parts; any other part (a `refusal`) fails
                    // its parse, which rejects the response as invalid JSON.
                    if let Some(other) = content_parts
                        .iter()
                        .find(|cp| cp.get("type").and_then(Value::as_str) != Some("output_text"))
                    {
                        return Err(AiMuxError::ApiCall(Box::new(ApiCallError {
                            status_code: Some(200),
                            response_body: Some(raw_body.to_string()),
                            response_headers: Some(response_headers.clone()),
                            ..ApiCallError::new(
                                format!(
                                    "Invalid JSON response: message content part of type {} is not output_text",
                                    other.get("type").unwrap_or(&Value::Null)
                                ),
                                request_url,
                                aimux_provider_utils::redact_error_context(body.clone()),
                            )
                        })));
                    }
                    for cp in content_parts {
                        if cp.get("type").and_then(|v| v.as_str()) == Some("output_text") {
                            let text = cp
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if body
                                .get("top_logprobs")
                                .and_then(Value::as_u64)
                                .is_some_and(|n| n > 0)
                                && let Some(value) = cp.get("logprobs")
                            {
                                logprobs.push(logprobs_fields(value));
                            }
                            {
                                content.push(GenerateContent::Text {
                                    text,
                                    provider_metadata: Some(
                                        provider_namespace(
                                            &provider_key,
                                            text_metadata(
                                                part,
                                                cp.get("annotations").and_then(Value::as_array),
                                            ),
                                        )
                                        .expect("provider metadata must be an object"),
                                    ),
                                });
                            }
                        }
                        if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array())
                        {
                            for ann in annotations {
                                content.push(GenerateContent::Source(annotation_source(
                                    ann,
                                    &provider_key,
                                )?));
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
                has_function_call = true;
                let call_id = part
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = part
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = part
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}")
                    .to_string();
                let input = arguments;
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: call_id,
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: Some(
                        provider_namespace(&provider_key, tool_metadata(part))
                            .expect("provider metadata must be an object"),
                    ),
                }));
            }
            Some("custom_tool_call") => {
                has_function_call = true;
                let call_id = part
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = part
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_str = part.get("input").and_then(|v| v.as_str()).unwrap_or("{}");
                let input = serde_json::to_string(input_str)
                    .expect("serializing a custom-tool input string cannot fail");
                content.push(GenerateContent::ToolCall(RawToolCall {
                    tool_call_id: call_id,
                    tool_name: name,
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: Some(
                        provider_namespace(&provider_key, tool_metadata(part))
                            .expect("provider metadata must be an object"),
                    ),
                }));
            }
            Some("reasoning") => {
                let summary = part.get("summary").and_then(|v| v.as_array());
                let parts: Vec<&Value> = summary.map(|s| s.iter().collect()).unwrap_or_default();
                let reasoning_metadata = Some(provider_namespace(
                    &provider_key,
                    json!({
                        "itemId": part.get("id").cloned().unwrap_or(Value::Null),
                        "reasoningEncryptedContent": part.get("encrypted_content").cloned().unwrap_or(Value::Null),
                    }),
                ).expect("provider metadata must be an object"));
                if parts.is_empty() {
                    content.push(GenerateContent::Reasoning(ReasoningOutput {
                        text: String::new(),
                        provider_metadata: reasoning_metadata.clone(),
                    }));
                } else {
                    for sp in parts {
                        let text = sp
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        content.push(GenerateContent::Reasoning(ReasoningOutput {
                            text,
                            provider_metadata: reasoning_metadata.clone(),
                        }));
                    }
                }
            }
            _ => {
                let mut part = part.clone();
                pair_tool_search_id(&mut part, &mut hosted_tool_search_ids);
                let parts = provider_tool_content(
                    &part,
                    &provider_key,
                    &tool_names,
                    shell_provider_executed,
                    part.get("approval_request_id")
                        .and_then(Value::as_str)
                        .and_then(|id| prompt_approval_tool_call_ids.get(id))
                        .map(String::as_str),
                );
                has_function_call |= part["type"] == "apply_patch_call"
                    || (part["type"] == "computer_call" && !part["call_id"].is_null());
                content.extend(parts);
            }
        }
    }

    let incomplete_reason = data
        .get("incomplete_details")
        .and_then(|d| d.get("reason"))
        .and_then(|v| v.as_str());
    let finish_reason = map_responses_finish_reason(incomplete_reason, has_function_call);

    let usage = convert_responses_usage(
        data.get("usage").and_then(parse_usage).as_ref(),
        data.get("usage").cloned(),
    );

    // Provider metadata: { <provider_key>: { responseId, reasoningContext?, serviceTier? } }
    let mut pm = json!({ "responseId": data.get("id").cloned().unwrap_or(Value::Null) });
    if let Some(reasoning) = data.get("reasoning")
        && let Some(ctx) = reasoning.get("context")
        && !ctx.is_null()
    {
        pm["reasoningContext"] = ctx.clone();
    }
    if let Some(st) = data.get("service_tier").and_then(|v| v.as_str()) {
        pm["serviceTier"] = json!(st);
    }
    if !logprobs.is_empty() {
        pm["logprobs"] = json!(logprobs);
    }
    let provider_metadata =
        Some(provider_namespace(&provider_key, pm).expect("provider metadata must be an object"));

    let response_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let model = data
        .get("model")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let created_at = data
        .get("created_at")
        .and_then(serde_json::Value::as_u64)
        .map(|secs| secs as f64);

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings: request_warnings,
        provider_metadata,
        response: Some(aimux_core::shared::ResponseInfo {
            headers: Some(response_headers),
            body: Some(data.clone()),
            ..aimux_provider_utils::create_language_model_response_metadata(
                response_id,
                model,
                created_at,
            )
            .into()
        }),
        request: Some(aimux_core::shared::RequestInfo { body: Some(body) }),
    })
}

// -- Streaming SSE event reducer ---------------------------------------------

/// A tool call being streamed (tracked by `output_index`).
struct OngoingToolCall {
    tool_call_id: String,
}

/// A reasoning item being streamed (tracked by `item_id`).
struct ReasoningState {
    encrypted_content: Option<String>,
    /// summary_index → status.
    summary_parts: HashMap<usize, SummaryStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SummaryStatus {
    Active,
    CanConclude,
    Concluded,
}

/// Provider metadata for streamed reasoning parts, in the same shape the
/// non-streaming path uses (`{ <provider_key>: { itemId,
/// reasoningEncryptedContent? } }`) so `consume()` can propagate it into
/// `response_messages` and the next turn's request conversion can read it
/// back (`item_reference` when stored, `encrypted_content` when not).
fn reasoning_stream_metadata(
    provider_key: &str,
    item_id: &str,
    encrypted_content: Option<&str>,
) -> ProviderMetadata {
    let inner = json!({ "itemId": item_id, "reasoningEncryptedContent": encrypted_content });
    provider_namespace(provider_key, inner).expect("provider metadata must be an object")
}

/// The error for a Chat Completions chunk on a Responses stream
/// (`createOpenAIResponsesChatCompletionsMismatchError`).
const CHAT_COMPLETIONS_MISMATCH: &str = concat!(
    "Received a Chat Completions stream while using the OpenAI Responses API. ",
    "The default OpenAI provider model uses the Responses API. If your custom baseURL targets a Chat Completions-compatible endpoint, use openai.chat('model-id') or createOpenAI(...).chat('model-id') instead. ",
    "You can also use @ai-sdk/openai-compatible for OpenAI-compatible providers.",
);

/// A Chat Completions chunk: a `choices` array and no string `type`
/// (`isOpenAIChatCompletionChunk`).
fn is_chat_completion_chunk(chunk: &Value) -> bool {
    chunk.get("choices").is_some_and(Value::is_array)
        && !chunk.get("type").is_some_and(Value::is_string)
}

/// Chunk types the AI SDK's chunk schema models
/// (`openaiResponsesModeledChunkTypes`); any other type is an unknown chunk.
const MODELED_CHUNK_TYPES: &[&str] = &[
    "error",
    "response.apply_patch_call_operation_diff.delta",
    "response.apply_patch_call_operation_diff.done",
    "response.code_interpreter_call_code.delta",
    "response.code_interpreter_call_code.done",
    "response.completed",
    "response.created",
    "response.custom_tool_call_input.delta",
    "response.failed",
    "response.function_call_arguments.delta",
    "response.function_call_arguments.done",
    "response.image_generation_call.partial_image",
    "response.in_progress",
    "response.incomplete",
    "response.output_item.added",
    "response.output_item.done",
    "response.output_text.annotation.added",
    "response.output_text.delta",
    "response.reasoning_summary_part.added",
    "response.reasoning_summary_part.done",
    "response.reasoning_summary_text.delta",
];

/// How long to keep looking for an error frame once `response.in_progress`
/// shows the request was accepted (`acceptedGraceMs`).
const ACCEPTED_GRACE: std::time::Duration = std::time::Duration::from_millis(50);

/// Reads the stream until its first output event, as the AI SDK's
/// `throwIfOpenAIStreamErrorBeforeOutput` does: an `error` frame, or a
/// `response.failed` with an error, before any output fails the call with
/// an `ApiCall` error instead of becoming an error part. After
/// `response.in_progress` each further read waits at most
/// [`ACCEPTED_GRACE`], so a healthy stream is handed over without waiting
/// for the first token. Returns the events read so far, to be replayed.
pub(crate) async fn read_until_output<S>(
    sse_stream: &mut S,
    request_url: &str,
    request_body: &Value,
    response_headers: &HashMap<String, String>,
) -> Result<Vec<Result<Value, AiMuxError>>, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin,
{
    let mut events = Vec::new();
    let mut accepted = false;
    loop {
        let next = if accepted {
            match tokio::time::timeout(ACCEPTED_GRACE, sse_stream.next()).await {
                Ok(next) => next,
                Err(_) => return Ok(events),
            }
        } else {
            sse_stream.next().await
        };
        let event = match next {
            None => return Ok(events),
            Some(Err(error @ AiMuxError::ApiCall(_))) => return Err(error),
            Some(Err(error)) => {
                events.push(Err(error));
                return Ok(events);
            }
            Some(Ok(event)) => event,
        };
        if is_chat_completion_chunk(&event) || chunk_schema_error(&event).is_some() {
            events.push(Ok(event));
            return Ok(events);
        }
        let kind = event["type"].as_str().unwrap_or_default();
        if kind == "error" || (kind == "response.failed" && !event["response"]["error"].is_null()) {
            return Err(crate::openai::stream_error::openai_stream_error(
                &event,
                request_url,
                request_body.clone(),
                response_headers.clone(),
            ));
        }
        let unknown_item = matches!(
            kind,
            "response.output_item.added" | "response.output_item.done"
        ) && !MODELED_OUTPUT_ITEM_TYPES
            .contains(&event["item"]["type"].as_str().unwrap_or_default());
        let output = MODELED_CHUNK_TYPES.contains(&kind)
            && !unknown_item
            && !matches!(
                kind,
                "response.created" | "response.in_progress" | "response.failed"
            );
        accepted |= kind == "response.in_progress";
        events.push(Ok(event));
        if output {
            return Ok(events);
        }
    }
}

/// Output item types the AI SDK's chunk schema models
/// (`openaiResponsesModeledOutputItemTypes`); an item event for another type
/// is an unknown chunk and is skipped.
const MODELED_OUTPUT_ITEM_TYPES: &[&str] = &[
    "apply_patch_call",
    "code_interpreter_call",
    "compaction",
    "computer_call",
    "custom_tool_call",
    "file_search_call",
    "function_call",
    "image_generation_call",
    "local_shell_call",
    "mcp_approval_request",
    "mcp_call",
    "mcp_list_tools",
    "message",
    "program",
    "program_output",
    "reasoning",
    "shell_call",
    "shell_call_output",
    "tool_search_call",
    "tool_search_output",
    "web_search_call",
];

/// Why a chunk fails the AI SDK's Responses chunk schema
/// (`openaiResponsesChunkSchema`): a chunk without a string `type`, or a
/// modeled event without the envelope fields its schema requires (the
/// `output_index`, `item_id`, `item`, delta or `response` of the event).
fn chunk_schema_error(chunk: &Value) -> Option<String> {
    let Some(kind) = chunk.get("type").and_then(Value::as_str) else {
        return Some("expected a string `type`".to_owned());
    };
    let (strings, numbers): (&[&str], &[&str]) = match kind {
        "response.output_item.added" | "response.output_item.done" => {
            let Some(item_type) = chunk
                .get("item")
                .and_then(Value::as_object)
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str)
            else {
                return Some("expected an `item` object with a string `type`".to_owned());
            };
            if !MODELED_OUTPUT_ITEM_TYPES.contains(&item_type) {
                return None;
            }
            (&[], &["output_index"])
        }
        "response.output_text.delta" => (&["item_id", "delta"], &[]),
        "response.function_call_arguments.delta"
        | "response.custom_tool_call_input.delta"
        | "response.code_interpreter_call_code.delta"
        | "response.apply_patch_call_operation_diff.delta" => {
            (&["item_id", "delta"], &["output_index"])
        }
        "response.function_call_arguments.done" => (&["item_id", "arguments"], &["output_index"]),
        "response.code_interpreter_call_code.done" => (&["item_id", "code"], &["output_index"]),
        "response.apply_patch_call_operation_diff.done" => {
            (&["item_id", "diff"], &["output_index"])
        }
        "response.image_generation_call.partial_image" => {
            (&["item_id", "partial_image_b64"], &["output_index"])
        }
        "response.reasoning_summary_part.added" | "response.reasoning_summary_part.done" => {
            (&["item_id"], &["summary_index"])
        }
        "response.reasoning_summary_text.delta" => (&["item_id", "delta"], &["summary_index"]),
        "response.created"
        | "response.in_progress"
        | "response.completed"
        | "response.incomplete"
        | "response.failed" => {
            if !chunk.get("response").is_some_and(Value::is_object) {
                return Some("expected a `response` object".to_owned());
            }
            (&[], &[])
        }
        _ => (&[], &[]),
    };
    if let Some(field) = strings
        .iter()
        .find(|field| !chunk.get(**field).is_some_and(Value::is_string))
    {
        return Some(format!("expected a string `{field}`"));
    }
    numbers
        .iter()
        .find(|field| !chunk.get(**field).is_some_and(Value::is_number))
        .map(|field| format!("expected a number `{field}`"))
}

/// The item id an event refers to: the id the output item at the event's
/// `output_index` started with, since OpenAI-compatible providers can rotate
/// item ids between events (`resolveOutputItemId`), else the event's own.
fn resolve_output_item_id(event: &Value, output_ids: &HashMap<usize, String>) -> String {
    event
        .get("output_index")
        .and_then(Value::as_u64)
        .and_then(|index| output_ids.get(&(index as usize)))
        .cloned()
        .unwrap_or_else(|| event["item_id"].as_str().unwrap_or_default().to_owned())
}

/// The metadata of a reasoning summary part that ends before its item does:
/// only the item id, without the encrypted content the item's end carries.
fn reasoning_item_metadata(provider_key: &str, item_id: &str) -> ProviderMetadata {
    provider_namespace(provider_key, json!({ "itemId": item_id }))
        .expect("provider metadata must be an object")
}

fn tool_metadata(item: &Value) -> Value {
    let mut metadata = json!({ "itemId": item.get("id").cloned().unwrap_or(Value::Null) });
    for key in ["async", "namespace"] {
        if let Some(value) = item.get(key).filter(|v| !v.is_null()) {
            metadata[key] = value.clone();
        }
    }
    if let Some(caller) = item.get("caller").filter(|value| !value.is_null()) {
        metadata["caller"] = if caller["type"] == "program" {
            json!({"type": "program", "callerId": caller["caller_id"]})
        } else {
            caller.clone()
        };
    }
    metadata
}

fn text_metadata(item: &Value, annotations: Option<&Vec<Value>>) -> Value {
    let mut metadata = json!({ "itemId": item.get("id").cloned().unwrap_or(Value::Null) });
    if let Some(phase) = item.get("phase").filter(|v| !v.is_null()) {
        metadata["phase"] = phase.clone();
    }
    if let Some(annotations) = annotations.filter(|v| !v.is_empty()) {
        metadata["annotations"] = annotations.iter().map(annotation_fields).collect();
    }
    metadata
}

/// Logprobs as the AI SDK's schema parses them: each token keeps `token`,
/// `logprob` and `top_logprobs` (each with `token` and `logprob`), so fields
/// such as `bytes` are dropped.
fn logprobs_fields(logprobs: &Value) -> Value {
    let Some(entries) = logprobs.as_array() else {
        return logprobs.clone();
    };
    let token = |entry: &Value| json!({"token": entry["token"], "logprob": entry["logprob"]});
    entries
        .iter()
        .map(|entry| {
            let mut output = token(entry);
            output["top_logprobs"] = entry["top_logprobs"]
                .as_array()
                .map(|top| top.iter().map(token).collect())
                .unwrap_or_else(|| entry["top_logprobs"].clone());
            output
        })
        .collect()
}

/// The fields the AI SDK's annotation schema keeps for each annotation type
/// (`openaiResponsesResponseSchema` and the `annotation.added` chunk schema);
/// any other field is dropped.
fn annotation_fields(annotation: &Value) -> Value {
    let fields: &[&str] = match annotation["type"].as_str() {
        Some("url_citation") => &["start_index", "end_index", "url", "title"],
        Some("file_citation") => &["file_id", "filename", "index"],
        Some("container_file_citation") => &[
            "container_id",
            "file_id",
            "filename",
            "start_index",
            "end_index",
        ],
        Some("file_path") => &["file_id", "index"],
        _ => return annotation.clone(),
    };
    let mut output = json!({"type": annotation["type"]});
    for field in fields {
        if let Some(value) = annotation.get(*field) {
            output[*field] = value.clone();
        }
    }
    output
}

fn annotation_source(annotation: &Value, provider_key: &str) -> Result<Source, AiMuxError> {
    let invalid = |field: &str| {
        AiMuxError::invalid_response_data(format!("Invalid Responses annotation field: {field}"))
    };
    let string = |field| {
        annotation
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(field))
    };
    let number = |field| {
        annotation
            .get(field)
            .filter(|value| value.is_number())
            .cloned()
            .ok_or_else(|| invalid(field))
    };
    let annotation_type = string("type")?;
    if annotation_type == "url_citation" {
        number("start_index")?;
        number("end_index")?;
        return Ok(Source::Url {
            id: generate_source_id(),
            url: string("url")?.to_string(),
            title: Some(string("title")?.to_string()),
            provider_metadata: None,
        });
    }
    let file_id = string("file_id")?;
    let (media_type, filename, metadata) = match annotation_type {
        "file_citation" | "file_path" => (
            if annotation_type == "file_path" {
                "application/octet-stream"
            } else {
                "text/plain"
            },
            if annotation_type == "file_path" {
                file_id
            } else {
                string("filename")?
            },
            json!({ "type": annotation_type, "fileId": file_id, "index": number("index")? }),
        ),
        "container_file_citation" => {
            number("start_index")?;
            number("end_index")?;
            (
                "text/plain",
                string("filename")?,
                json!({
                    "type": annotation_type,
                    "fileId": file_id,
                    "containerId": string("container_id")?,
                }),
            )
        }
        _ => return Err(invalid("type")),
    };
    Ok(Source::Document {
        id: generate_source_id(),
        media_type: media_type.to_string(),
        title: filename.to_string(),
        filename: Some(filename.to_string()),
        provider_metadata: Some(provider_namespace(provider_key, metadata)?),
    })
}

/// Generate a unique source ID for streaming annotation sources.
///
/// Uses a process-wide atomic counter — consistent with the xAI Responses
/// provider (`aimux-providers/src/xai/responses/responses_language_model.rs`). Upstream TS uses
/// `generateId()` for the same purpose.
fn generate_source_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("source-{n}")
}

/// Build the streaming event reducer shared by the OpenAI and Azure providers.
///
/// Both speak the same Responses streaming wire format (the
/// `response.created -> output_item.added -> output_text.delta ->
/// output_item.done -> response.completed` main path, plus
/// `function_call_arguments.delta`, `custom_tool_call_input.delta`,
/// `reasoning_summary_part.added/done` and `reasoning_summary_text.delta`).
///
/// The caller performs the API call and hands the peeked
/// `first_event` plus the remainder `sse_stream` to this reducer; an early
/// `error` / `response.failed` surfaces as a clean `Err` here.
///
/// # Errors
///
/// An early `error` / `response.failed` event returns an `ApiCall` setup error;
/// malformed events yield parse-error items and do not end the stream.
// This reducer is shared by OpenAI, Azure, and Codex. Keeping the wire inputs
// explicit is clearer than introducing a second context object used nowhere
// else.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_event_stream<S>(
    first_event: Option<Result<Value, AiMuxError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin + Send + 'static,
{
    build_responses_event_stream_with_raw(
        first_event,
        sse_stream,
        provider_key,
        warnings,
        store_flag,
        request_url,
        request_body,
        response_headers,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_event_stream_with_raw<S>(
    first_event: Option<Result<Value, AiMuxError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
    include_raw_chunks: bool,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin + Send + 'static,
{
    build_responses_event_stream_with_tools(
        first_event,
        sse_stream,
        provider_key,
        warnings,
        store_flag,
        request_url,
        request_body,
        response_headers,
        include_raw_chunks,
        HashMap::new(),
        HashMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_responses_event_stream_with_tools<S>(
    first_event: Option<Result<Value, AiMuxError>>,
    sse_stream: S,
    provider_key: String,
    warnings: Vec<Warning>,
    store_flag: bool,
    request_url: String,
    request_body: Value,
    response_headers: HashMap<String, String>,
    include_raw_chunks: bool,
    tool_names: HashMap<String, String>,
    prompt_approval_tool_call_ids: HashMap<String, String>,
) -> Result<ResponsesEventStream, AiMuxError>
where
    S: Stream<Item = Result<Value, AiMuxError>> + Unpin + Send + 'static,
{
    // Peek at the first SSE event to detect early errors (before any output).
    if let Some(Ok(ref val)) = first_event {
        let etype = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if etype == "error" || etype == "response.failed" {
            return Err(crate::openai::stream_error::openai_stream_error(
                val,
                &request_url,
                request_body.clone(),
                response_headers.clone(),
            ));
        }
    }

    let shell_provider_executed = hosted_shell(&request_body);
    let stream = pipe_through(
        futures::stream::iter(first_event).chain(sse_stream),
        OpenAIResponsesStream {
            ctx: StreamContext {
                warnings,
                include_raw_chunks,
                url: request_url,
                request_body,
                response_headers,
                ..StreamContext::default()
            },
            provider_key,
            store_flag,
            tool_names,
            prompt_approval_tool_call_ids,
            shell_provider_executed,
            ..OpenAIResponsesStream::default()
        },
    );
    Ok(Box::pin(stream))
}

/// The `TransformStream` of the OpenAI Responses `doStream`.
#[derive(Default)]
struct OpenAIResponsesStream {
    ctx: StreamContext,
    provider_key: String,
    store_flag: bool,
    tool_names: HashMap<String, String>,
    prompt_approval_tool_call_ids: HashMap<String, String>,
    shell_provider_executed: bool,
    /// Ongoing tool calls keyed by output_index.
    ongoing_tool_calls: HashMap<usize, OngoingToolCall>,
    /// Active reasoning items keyed by item_id.
    active_reasoning: HashMap<String, ReasoningState>,
    has_function_call: bool,
    final_usage: Option<ResponsesUsage>,
    final_raw_usage: Option<Value>,
    final_service_tier: Option<String>,
    final_reasoning_context: Option<Value>,
    final_finish_reason: Option<FinishReason>,
    response_id: Option<String>,
    stream_errored: bool,
    text_states: HashMap<String, Value>,
    output_ids: HashMap<usize, String>,
    text_annotations: HashMap<String, Vec<Value>>,
    logprobs: Vec<Value>,
    provider_items: HashMap<usize, Value>,
    emitted_calls: HashSet<String>,
    ended_inputs: HashSet<String>,
    diff_received: HashSet<String>,
    hosted_tool_search_ids: VecDeque<Value>,
    approval_tool_call_ids: HashMap<String, String>,
}

impl Transformer for OpenAIResponsesStream {
    type Input = Result<Value, AiMuxError>;
    type Output = StreamPart;

    fn start(&mut self, controller: &mut Controller) {
        self.ctx.start(controller);
    }

    fn transform(&mut self, event: Result<Value, AiMuxError>, controller: &mut Controller) {
        let Self {
            ref provider_key,
            store_flag,
            ctx:
                StreamContext {
                    ref url,
                    ref request_body,
                    ref response_headers,
                    include_raw_chunks,
                    ..
                },
            ref tool_names,
            ref prompt_approval_tool_call_ids,
            shell_provider_executed,
            ref mut ongoing_tool_calls,
            ref mut active_reasoning,
            ref mut has_function_call,
            ref mut final_usage,
            ref mut final_raw_usage,
            ref mut final_service_tier,
            ref mut final_reasoning_context,
            ref mut final_finish_reason,
            ref mut response_id,
            ref mut stream_errored,
            ref mut text_states,
            ref mut output_ids,
            ref mut text_annotations,
            ref mut logprobs,
            ref mut provider_items,
            ref mut emitted_calls,
            ref mut ended_inputs,
            ref mut diff_received,
            ref mut hosted_tool_search_ids,
            ref mut approval_tool_call_ids,
            ..
        } = *self;
        match event {
            Ok(parsed) => {
                if include_raw_chunks {
                    controller.enqueue(StreamPart::Raw {
                        raw_value: parsed.clone(),
                    });
                }
                if is_chat_completion_chunk(&parsed) {
                    *stream_errored = true;
                    *final_finish_reason = Some(FINISH_ERROR);
                    controller.enqueue(StreamPart::Error {
                        error: aimux_provider_utils::create_provider_stream_error(
                            aimux_provider_utils::ProviderStreamError {
                                message: CHAT_COMPLETIONS_MISMATCH.to_owned(),
                                data: parsed.clone(),
                                ..Default::default()
                            },
                            &parsed,
                            url.clone(),
                            request_body.clone(),
                            response_headers.clone(),
                        ),
                    });
                    return;
                }
                if let Some(detail) = chunk_schema_error(&parsed) {
                    *stream_errored = true;
                    *final_finish_reason = Some(FINISH_ERROR);
                    controller.enqueue(StreamPart::Error {
                        error: AiMuxError::TypeValidation {
                            message: format!(
                                "Type validation failed: Value: {parsed}.\nError message: {detail}"
                            ),
                            value: Box::new(parsed),
                            cause: None,
                        },
                    });
                    return;
                }
                let etype = parsed
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                match etype.as_str() {
                    // ── response.created → ResponseMetadata ───────────────────
                    "response.created" => {
                        if let Some(resp_obj) = parsed.get("response") {
                            *response_id = resp_obj
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let model_id = resp_obj
                                .get("model")
                                .and_then(|v| v.as_str())
                                .map(std::string::ToString::to_string);
                            let created_at = resp_obj
                                .get("created_at")
                                .and_then(serde_json::Value::as_u64)
                                .map(|secs| secs as f64);
                            controller.enqueue(StreamPart::ResponseMetadata(
                                aimux_provider_utils::create_language_model_response_metadata(
                                    response_id.clone(),
                                    model_id,
                                    created_at,
                                ),
                            ));
                        }
                    }

                    // ── response.output_item.added ─────────────────────────────
                    "response.output_item.added" => {
                        let output_index = parsed
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        if let Some(item) = parsed.get("item") {
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match item_type {
                                "message" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    output_ids.insert(output_index, id.clone());
                                    text_states.insert(id.clone(), item.clone());
                                    controller.enqueue(StreamPart::TextStart {
                                        id,
                                        provider_metadata: Some(
                                            provider_namespace(
                                                provider_key,
                                                text_metadata(item, None),
                                            )
                                            .expect("provider metadata must be an object"),
                                        ),
                                    });
                                }
                                "function_call" => {
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    ongoing_tool_calls.insert(
                                        output_index,
                                        OngoingToolCall {
                                            tool_call_id: call_id.clone(),
                                        },
                                    );
                                    controller.tool_input_start(call_id, name);
                                }
                                "custom_tool_call" => {
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    ongoing_tool_calls.insert(
                                        output_index,
                                        OngoingToolCall {
                                            tool_call_id: call_id.clone(),
                                        },
                                    );
                                    controller.tool_input_start(call_id, name);
                                }
                                "reasoning" => {
                                    let id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let encrypted = item
                                        .get("encrypted_content")
                                        .and_then(|v| v.as_str())
                                        .map(std::string::ToString::to_string);
                                    // Carry itemId + encrypted_content in
                                    // provider_metadata (same shape as the
                                    // non-streaming path) so reasoning can
                                    // be echoed back on the next turn.
                                    let meta = reasoning_stream_metadata(
                                        provider_key,
                                        &id,
                                        encrypted.as_deref(),
                                    );
                                    output_ids.insert(output_index, id.clone());
                                    active_reasoning.insert(
                                        id.clone(),
                                        ReasoningState {
                                            encrypted_content: encrypted,
                                            summary_parts: HashMap::from([(
                                                0usize,
                                                SummaryStatus::Active,
                                            )]),
                                        },
                                    );
                                    controller.enqueue(StreamPart::ReasoningStart {
                                        id: format!("{id}:0"),
                                        provider_metadata: Some(meta),
                                    });
                                }
                                _ => {
                                    provider_items.insert(output_index, item.clone());
                                    for part in provider_tool_start(item, tool_names) {
                                        if let StreamPart::ToolCall(call) = &part {
                                            emitted_calls.insert(call.tool_call_id.clone());
                                        }
                                        if let StreamPart::ToolInputEnd { id, .. } = &part {
                                            ended_inputs.insert(id.clone());
                                        }
                                        controller.enqueue(part);
                                    }
                                }
                            }
                        }
                    }

                    "response.image_generation_call.partial_image" => {
                        controller.enqueue(StreamPart::ToolResult(aimux_core::tool::ToolResult {
                            tool_call_id: parsed["item_id"].as_str().unwrap_or_default().to_owned(),
                            tool_name: tool_names
                                .get("image_generation")
                                .map_or("image_generation", String::as_str)
                                .to_owned(),
                            result: json!({"result": parsed["partial_image_b64"]}),
                            preliminary: Some(true),
                            is_error: None,
                            dynamic: None,
                            provider_metadata: None,
                        }));
                    }
                    "response.code_interpreter_call_code.delta"
                    | "response.apply_patch_call_operation_diff.delta" => {
                        let index = parsed["output_index"].as_u64().unwrap_or(0) as usize;
                        if let Some(item) = provider_items.get(&index) {
                            let id = item
                                .get("call_id")
                                .filter(|v| !v.is_null())
                                .unwrap_or(&item["id"])
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            diff_received.insert(id.clone());
                            controller.tool_input_delta(
                                id,
                                escaped_delta(parsed["delta"].as_str().unwrap_or_default()),
                            );
                        }
                    }
                    "response.code_interpreter_call_code.done"
                    | "response.apply_patch_call_operation_diff.done" => {
                        let index = parsed["output_index"].as_u64().unwrap_or(0) as usize;
                        if let Some(item) = provider_items.get(&index) {
                            let id = item
                                .get("call_id")
                                .filter(|v| !v.is_null())
                                .unwrap_or(&item["id"])
                                .as_str()
                                .unwrap_or_default()
                                .to_owned();
                            if ended_inputs.insert(id.clone()) {
                                let code = etype == "response.code_interpreter_call_code.done";
                                if !code && !diff_received.contains(&id) {
                                    controller.tool_input_delta(
                                        id.clone(),
                                        escaped_delta(parsed["diff"].as_str().unwrap_or_default()),
                                    );
                                }
                                controller.tool_input_delta(
                                    id.clone(),
                                    if code { "\"}" } else { "\"}}" }.to_owned(),
                                );
                                controller.tool_input_end(id.clone());
                                if code {
                                    let mut item = item.clone();
                                    item["code"] = parsed["code"].clone();
                                    for part in provider_tool_content(
                                        &item,
                                        provider_key,
                                        tool_names,
                                        shell_provider_executed,
                                        None,
                                    ) {
                                        if let GenerateContent::ToolCall(call) = part {
                                            emitted_calls.insert(id.clone());
                                            controller.enqueue(StreamPart::ToolCall(call));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // ── response.output_text.delta → TextDelta ────────────────
                    "response.output_text.delta" => {
                        if request_body
                            .get("top_logprobs")
                            .and_then(Value::as_u64)
                            .is_some_and(|n| n > 0)
                            && let Some(value) = parsed.get("logprobs")
                        {
                            logprobs.push(logprobs_fields(value));
                        }
                        let mut id = parsed
                            .get("item_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(index) = parsed.get("output_index").and_then(Value::as_u64)
                            && let Some(original_id) = output_ids.get(&(index as usize))
                        {
                            id = original_id.clone();
                        }
                        let delta = parsed
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        controller.text_delta(id, delta);
                    }

                    // ── function_call_arguments.delta → ToolInputDelta ────────
                    "response.function_call_arguments.delta"
                    | "response.custom_tool_call_input.delta" => {
                        let output_index = parsed
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        let delta = parsed
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(tc) = ongoing_tool_calls.get(&output_index) {
                            controller.tool_input_delta(tc.tool_call_id.clone(), delta);
                        }
                    }

                    // ── reasoning_summary_part.added ──────────────────────────
                    "response.reasoning_summary_part.added" => {
                        let item_id = resolve_output_item_id(&parsed, output_ids);
                        let summary_index = parsed
                            .get("summary_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;

                        if summary_index > 0
                            && let Some(state) = active_reasoning.get_mut(&item_id)
                        {
                            let meta = reasoning_stream_metadata(
                                provider_key,
                                &item_id,
                                state.encrypted_content.as_deref(),
                            );
                            // Conclude all 'can-conclude' parts.
                            let to_conclude: Vec<usize> = state
                                .summary_parts
                                .iter()
                                .filter(|(_, s)| **s == SummaryStatus::CanConclude)
                                .map(|(k, _)| *k)
                                .collect();
                            for idx in to_conclude {
                                state.summary_parts.insert(idx, SummaryStatus::Concluded);
                                controller.enqueue(StreamPart::ReasoningEnd {
                                    id: format!("{item_id}:{idx}"),
                                    provider_metadata: Some(reasoning_item_metadata(
                                        provider_key,
                                        &item_id,
                                    )),
                                });
                            }
                            state
                                .summary_parts
                                .insert(summary_index, SummaryStatus::Active);
                            controller.enqueue(StreamPart::ReasoningStart {
                                id: format!("{item_id}:{summary_index}"),
                                provider_metadata: Some(meta),
                            });
                        }
                    }

                    // ── reasoning_summary_text.delta → ReasoningDelta ─────────
                    "response.reasoning_summary_text.delta" => {
                        let item_id = resolve_output_item_id(&parsed, output_ids);
                        let summary_index = parsed
                            .get("summary_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        let delta = parsed
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        controller.enqueue(StreamPart::ReasoningDelta {
                            id: format!("{item_id}:{summary_index}"),
                            delta,
                            provider_metadata: Some(
                                provider_namespace(provider_key, json!({ "itemId": item_id }))
                                    .expect("provider metadata must be an object"),
                            ),
                        });
                    }

                    // ── reasoning_summary_part.done ───────────────────────────
                    "response.reasoning_summary_part.done" => {
                        let item_id = resolve_output_item_id(&parsed, output_ids);
                        let summary_index = parsed
                            .get("summary_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        if let Some(state) = active_reasoning.get_mut(&item_id) {
                            if store_flag {
                                state
                                    .summary_parts
                                    .insert(summary_index, SummaryStatus::Concluded);
                                controller.enqueue(StreamPart::ReasoningEnd {
                                    id: format!("{item_id}:{summary_index}"),
                                    provider_metadata: Some(reasoning_item_metadata(
                                        provider_key,
                                        &item_id,
                                    )),
                                });
                            } else {
                                state
                                    .summary_parts
                                    .insert(summary_index, SummaryStatus::CanConclude);
                            }
                        }
                    }

                    // ── response.output_item.done ─────────────────────────────
                    "response.output_item.done" => {
                        let output_index = parsed
                            .get("output_index")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        if let Some(item) = parsed.get("item") {
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match item_type {
                                "message" => {
                                    let mut id = item
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if let Some(original_id) = output_ids.remove(&output_index) {
                                        id = original_id;
                                    }
                                    let mut metadata_item = item.clone();
                                    metadata_item["id"] = json!(id);
                                    if metadata_item.get("phase").is_none()
                                        && let Some(phase) =
                                            text_states.get(&id).and_then(|v| v.get("phase"))
                                    {
                                        metadata_item["phase"] = phase.clone();
                                    }
                                    let annotations = text_annotations.remove(&id);
                                    controller.enqueue(StreamPart::TextEnd {
                                        id,
                                        provider_metadata: Some(
                                            provider_namespace(
                                                provider_key,
                                                text_metadata(&metadata_item, annotations.as_ref()),
                                            )
                                            .expect("provider metadata must be an object"),
                                        ),
                                    });
                                }
                                "function_call" => {
                                    *has_function_call = true;
                                    ongoing_tool_calls.remove(&output_index);
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let arguments = item
                                        .get("arguments")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("{}")
                                        .to_string();
                                    controller.enqueue(StreamPart::ToolInputEnd {
                                        id: call_id.clone(),
                                        provider_metadata: item
                                            .get("namespace")
                                            .filter(|namespace| !namespace.is_null())
                                            .map(|namespace| {
                                                provider_namespace(
                                                    provider_key,
                                                    json!({ "namespace": namespace }),
                                                )
                                                .expect("provider metadata must be an object")
                                            }),
                                    });
                                    let input = arguments;
                                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: call_id,
                                        tool_name: name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        provider_metadata: Some(
                                            provider_namespace(provider_key, tool_metadata(item))
                                                .expect("provider metadata must be an object"),
                                        ),
                                    }));
                                }
                                "custom_tool_call" => {
                                    *has_function_call = true;
                                    ongoing_tool_calls.remove(&output_index);
                                    let call_id = item
                                        .get("call_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let name = item
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let input_str =
                                        item.get("input").and_then(|v| v.as_str()).unwrap_or("{}");
                                    controller.tool_input_end(call_id.clone());
                                    let input = serde_json::to_string(input_str).expect(
                                        "serializing a custom-tool input string cannot fail",
                                    );
                                    controller.enqueue(StreamPart::ToolCall(RawToolCall {
                                        tool_call_id: call_id,
                                        tool_name: name,
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        provider_metadata: Some(
                                            provider_namespace(provider_key, tool_metadata(item))
                                                .expect("provider metadata must be an object"),
                                        ),
                                    }));
                                }
                                "reasoning" => {
                                    let id =
                                        output_ids.remove(&output_index).unwrap_or_else(|| {
                                            item.get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string()
                                        });
                                    if let Some(state) = active_reasoning.get_mut(&id) {
                                        state.encrypted_content = item
                                            .get("encrypted_content")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned);
                                        let meta = reasoning_stream_metadata(
                                            provider_key,
                                            &id,
                                            state.encrypted_content.as_deref(),
                                        );
                                        // Conclude all active / can-conclude parts.
                                        let to_conclude: Vec<usize> = state
                                            .summary_parts
                                            .iter()
                                            .filter(|(_, s)| {
                                                **s == SummaryStatus::Active
                                                    || **s == SummaryStatus::CanConclude
                                            })
                                            .map(|(k, _)| *k)
                                            .collect();
                                        for idx in to_conclude {
                                            state
                                                .summary_parts
                                                .insert(idx, SummaryStatus::Concluded);
                                            controller.enqueue(StreamPart::ReasoningEnd {
                                                id: format!("{id}:{idx}"),
                                                provider_metadata: Some(meta.clone()),
                                            });
                                        }
                                    }
                                    active_reasoning.remove(&id);
                                }
                                _ => {
                                    let mut item = item.clone();
                                    if item_type == "local_shell_call"
                                        && let Some(action) = item["action"].as_object_mut()
                                    {
                                        for (from, to) in [
                                            ("timeout_ms", "timeoutMs"),
                                            ("working_directory", "workingDirectory"),
                                        ] {
                                            if let Some(value) = action.remove(from) {
                                                action.insert(to.into(), value);
                                            }
                                        }
                                    }
                                    if item_type == "apply_patch_call" {
                                        let id =
                                            item["call_id"].as_str().unwrap_or_default().to_owned();
                                        if ended_inputs.insert(id.clone())
                                            && item["operation"]["type"] != "delete_file"
                                        {
                                            if !diff_received.contains(&id) {
                                                controller.tool_input_delta(
                                                    id.clone(),
                                                    escaped_delta(
                                                        item["operation"]["diff"]
                                                            .as_str()
                                                            .unwrap_or_default(),
                                                    ),
                                                );
                                            }
                                            controller.tool_input_delta(id.clone(), "\"}}");
                                            controller.tool_input_end(id);
                                        }
                                    }
                                    if item_type == "tool_search_call"
                                        && item["execution"] == "server"
                                    {
                                        item["call_id"] = Value::Null;
                                    }
                                    pair_tool_search_id(&mut item, hosted_tool_search_ids);
                                    let mcp_tool_call_id = item
                                        .get("approval_request_id")
                                        .and_then(Value::as_str)
                                        .and_then(|id| {
                                            approval_tool_call_ids
                                                .get(id)
                                                .or_else(|| prompt_approval_tool_call_ids.get(id))
                                        })
                                        .cloned();
                                    for part in provider_tool_content(
                                        &item,
                                        provider_key,
                                        tool_names,
                                        shell_provider_executed,
                                        mcp_tool_call_id.as_deref(),
                                    ) {
                                        match part {
                                            GenerateContent::ToolCall(call) => {
                                                if item_type == "code_interpreter_call"
                                                    || (item_type == "apply_patch_call"
                                                        && item["status"] != "completed")
                                                {
                                                    continue;
                                                }
                                                if emitted_calls.insert(call.tool_call_id.clone())
                                                    || item_type == "mcp_call"
                                                {
                                                    *has_function_call |= call.provider_executed
                                                        != Some(true)
                                                        && matches!(
                                                            item_type,
                                                            "computer_call" | "apply_patch_call"
                                                        );
                                                    if matches!(
                                                        item_type,
                                                        "computer_call"
                                                            | "apply_patch_call"
                                                            | "tool_search_call"
                                                    ) {
                                                        if item_type == "tool_search_call"
                                                            && item["execution"] != "server"
                                                        {
                                                            controller.enqueue(
                                                                StreamPart::ToolInputStart {
                                                                    id: call.tool_call_id.clone(),
                                                                    tool_name: call
                                                                        .tool_name
                                                                        .clone(),
                                                                    provider_executed: call
                                                                        .provider_executed,
                                                                    dynamic: None,
                                                                    title: None,
                                                                    provider_metadata: None,
                                                                },
                                                            );
                                                        }
                                                        if !ended_inputs
                                                            .contains(&call.tool_call_id)
                                                        {
                                                            if item_type == "computer_call"
                                                                && !call.input.is_empty()
                                                            {
                                                                controller.enqueue(
                                                                    StreamPart::ToolInputDelta {
                                                                        id: call
                                                                            .tool_call_id
                                                                            .clone(),
                                                                        delta: call.input.clone(),
                                                                        provider_metadata: None,
                                                                    },
                                                                );
                                                            }
                                                            controller.enqueue(
                                                                StreamPart::ToolInputEnd {
                                                                    id: call.tool_call_id.clone(),
                                                                    provider_metadata: None,
                                                                },
                                                            );
                                                        }
                                                    }
                                                    controller.enqueue(StreamPart::ToolCall(call));
                                                }
                                            }
                                            GenerateContent::ToolResult(result) => {
                                                controller.enqueue(StreamPart::ToolResult(result));
                                            }
                                            GenerateContent::ToolApprovalRequest(approval) => {
                                                approval_tool_call_ids.insert(
                                                    approval.approval_id.clone(),
                                                    approval.tool_call_id.clone(),
                                                );
                                                controller.enqueue(
                                                    StreamPart::ToolApprovalRequest(approval),
                                                );
                                            }
                                            GenerateContent::Custom {
                                                kind,
                                                provider_metadata,
                                            } => {
                                                controller.enqueue(StreamPart::Custom {
                                                    kind,
                                                    provider_metadata,
                                                });
                                            }
                                            _ => {}
                                        }
                                    }
                                    provider_items.remove(&output_index);
                                }
                            }
                        }
                    }

                    // ── response.output_text.annotation.added → Source ─────────
                    "response.output_text.annotation.added" => {
                        match annotation_source(
                            parsed.get("annotation").unwrap_or(&Value::Null),
                            provider_key,
                        ) {
                            Ok(source) => {
                                let id = parsed
                                    .get("item_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                text_annotations
                                    .entry(id)
                                    .or_default()
                                    .push(parsed["annotation"].clone());
                                controller.enqueue(StreamPart::Source(source));
                            }
                            Err(error) => {
                                *stream_errored = true;
                                controller.enqueue(StreamPart::Error { error });
                            }
                        }
                    }

                    // ── response.completed / response.incomplete → finish ────
                    "response.completed" | "response.incomplete" => {
                        if let Some(resp_obj) = parsed.get("response") {
                            let reason = resp_obj
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|v| v.as_str());
                            if !*stream_errored {
                                *final_finish_reason =
                                    Some(map_responses_finish_reason(reason, *has_function_call));
                            }
                            *final_usage = resp_obj.get("usage").and_then(parse_usage);
                            // Keep the raw wire usage object for `usage.raw`
                            // (RFC-0015 P0-3) and capture the finish-time
                            // provider metadata fields carried on the
                            // terminal response object.
                            *final_raw_usage = resp_obj.get("usage").cloned();
                            if let Some(st) = resp_obj.get("service_tier").and_then(|v| v.as_str())
                            {
                                *final_service_tier = Some(st.to_string());
                            }
                            if let Some(reasoning) = resp_obj.get("reasoning")
                                && let Some(ctx) = reasoning.get("context")
                                && !ctx.is_null()
                            {
                                *final_reasoning_context = Some(ctx.clone());
                            }
                        }
                    }

                    // ── response.failed ───────────────────────────────────────
                    "response.failed" => {
                        if let Some(resp_obj) = parsed.get("response") {
                            let reason = resp_obj
                                .get("incomplete_details")
                                .and_then(|d| d.get("reason"))
                                .and_then(|v| v.as_str());
                            *final_finish_reason = Some(match reason {
                                Some(r) => map_responses_finish_reason(Some(r), *has_function_call),
                                None => FinishReason {
                                    unified: FinishReasonUnified::Error,
                                    raw: Some("error".to_string()),
                                },
                            });
                            *final_usage = resp_obj.get("usage").and_then(parse_usage);
                            *final_raw_usage = resp_obj.get("usage").cloned();
                            // Upstream's `response.failed` arm carries
                            // `reasoningContext` (but not `serviceTier`).
                            if let Some(reasoning) = resp_obj.get("reasoning")
                                && let Some(ctx) = reasoning.get("context")
                                && !ctx.is_null()
                            {
                                *final_reasoning_context = Some(ctx.clone());
                            }
                            if !*stream_errored && resp_obj.get("error").is_some() {
                                *stream_errored = true;
                                controller.enqueue(StreamPart::Error {
                                    error: crate::openai::stream_error::openai_stream_error(
                                        &parsed,
                                        url,
                                        request_body.clone(),
                                        response_headers.clone(),
                                    ),
                                });
                            }
                        }
                    }

                    // ── error chunk ───────────────────────────────────────────
                    "error" => {
                        *stream_errored = true;
                        *final_finish_reason = Some(FinishReason {
                            unified: FinishReasonUnified::Error,
                            raw: Some("error".to_string()),
                        });
                        controller.enqueue(StreamPart::Error {
                            error: crate::openai::stream_error::openai_stream_error(
                                &parsed,
                                url,
                                request_body.clone(),
                                response_headers.clone(),
                            ),
                        });
                    }

                    _ => {
                        // Unknown / unhandled chunk types (content_part.added,
                        // output_text.done, content_part.done, etc.) are
                        // ignored — the core streaming path is driven by
                        // output_item.added/done and the delta events.
                    }
                }
            }
            Err(error) => {
                *stream_errored = true;
                *final_finish_reason = Some(FINISH_ERROR);
                controller.enqueue(StreamPart::Error { error });
            }
        }
    }

    fn flush(self, controller: &mut Controller) {
        // Build provider metadata for the Finish part: { <provider_key>: {
        // responseId, serviceTier?, reasoningContext? } }. Mirrors the TS
        // flush() providerMetadata, which carries `service_tier` and
        // `reasoning.context` from the terminal response object.
        let mut pm = json!({ "responseId": self.response_id });
        if let Some(st) = self.final_service_tier {
            pm["serviceTier"] = json!(st);
        }
        if let Some(ctx) = self.final_reasoning_context {
            pm["reasoningContext"] = ctx;
        }
        if !self.logprobs.is_empty() {
            pm["logprobs"] = json!(self.logprobs);
        }
        let provider_metadata = Some(
            provider_namespace(&self.provider_key, pm)
                .expect("provider metadata must be an object"),
        );

        controller.enqueue(StreamPart::Finish {
            finish_reason: self.final_finish_reason.unwrap_or(FINISH_OTHER),
            usage: convert_responses_usage(self.final_usage.as_ref(), self.final_raw_usage),
            provider_metadata,
        });
    }
}

fn pair_tool_search_id(item: &mut Value, hosted_ids: &mut VecDeque<Value>) {
    match item["type"].as_str() {
        Some("tool_search_call") if item["execution"] == "server" => {
            hosted_ids.push_back(
                item.get("call_id")
                    .filter(|v| !v.is_null())
                    .unwrap_or(&item["id"])
                    .clone(),
            );
        }
        Some("tool_search_output") if item.get("call_id").is_none_or(Value::is_null) => {
            if let Some(id) = hosted_ids.pop_front() {
                item["call_id"] = id;
            }
        }
        _ => {}
    }
}

fn escaped_delta(value: &str) -> String {
    let value = json!(value).to_string();
    value[1..value.len() - 1].to_owned()
}

fn provider_tool_start(item: &Value, names: &HashMap<String, String>) -> Vec<StreamPart> {
    let kind = item["type"].as_str().unwrap_or_default();
    let canonical = match kind {
        "web_search_call" => "web_search",
        "file_search_call" => "file_search",
        "image_generation_call" => "image_generation",
        "code_interpreter_call" => "code_interpreter",
        "computer_call" => "computer",
        "apply_patch_call" => "apply_patch",
        "tool_search_call" if item["execution"] == "server" => "tool_search",
        _ => return Vec::new(),
    };
    let name = names
        .get(canonical)
        .map_or(canonical, String::as_str)
        .to_owned();
    let id = item
        .get("call_id")
        .filter(|v| !v.is_null())
        .unwrap_or(&item["id"])
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let hosted = matches!(
        kind,
        "web_search_call"
            | "file_search_call"
            | "image_generation_call"
            | "code_interpreter_call"
            | "tool_search_call"
    )
    .then_some(true);
    let call = || {
        StreamPart::ToolCall(RawToolCall {
            tool_call_id: id.clone(),
            tool_name: name.clone(),
            input: "{}".to_owned(),
            provider_executed: hosted,
            dynamic: None,
            provider_metadata: None,
        })
    };
    if matches!(kind, "file_search_call" | "image_generation_call") {
        return vec![call()];
    }
    let mut parts = vec![StreamPart::ToolInputStart {
        id: id.clone(),
        tool_name: name.clone(),
        provider_executed: hosted,
        dynamic: None,
        title: None,
        provider_metadata: None,
    }];
    match kind {
        "web_search_call" => {
            parts.push(StreamPart::ToolInputEnd {
                id: id.clone(),
                provider_metadata: None,
            });
            parts.push(call());
        }
        "code_interpreter_call" => parts.push(StreamPart::ToolInputDelta {
            id,
            delta: format!("{{\"containerId\":{},\"code\":\"", item["container_id"]),
            provider_metadata: None,
        }),
        "apply_patch_call" => {
            let operation = &apply_patch_operation(&item["operation"]);
            if operation["type"] == "delete_file" {
                parts.push(StreamPart::ToolInputDelta {
                    id: id.clone(),
                    delta: json!({"callId":id,"operation":operation}).to_string(),
                    provider_metadata: None,
                });
                parts.push(StreamPart::ToolInputEnd {
                    id,
                    provider_metadata: None,
                });
            } else {
                parts.push(StreamPart::ToolInputDelta {
                    id: id.clone(),
                    delta: format!(
                        "{{\"callId\":{},\"operation\":{{\"type\":{},\"path\":{},\"diff\":\"",
                        json!(id),
                        operation["type"],
                        operation["path"]
                    ),
                    provider_metadata: None,
                });
            }
        }
        _ => {}
    }
    parts
}

fn hosted_shell(body: &Value) -> bool {
    body["tools"].as_array().is_some_and(|tools| {
        tools.iter().any(|tool| {
            tool["type"] == "shell"
                && tool.get("environment").is_some_and(|environment| {
                    !matches!(environment["type"].as_str(), Some("local"))
                })
        })
    })
}

// -- Provider tool output items ----------------------------------------------

pub(crate) fn provider_tool_content(
    part: &Value,
    provider_key: &str,
    tool_names: &HashMap<String, String>,
    shell_provider_executed: bool,
    mcp_tool_call_id: Option<&str>,
) -> Vec<GenerateContent> {
    let kind = part["type"].as_str().unwrap_or_default();
    let item_id = part["id"].as_str().unwrap_or_default();
    let call_id = part["call_id"].as_str().unwrap_or(item_id);
    let metadata = || {
        Some(
            provider_namespace(provider_key, json!({"itemId": item_id}))
                .expect("provider metadata must be an object"),
        )
    };
    let name = |canonical: &str| {
        tool_names
            .get(canonical)
            .cloned()
            .unwrap_or_else(|| canonical.to_string())
    };
    let call = |id: &str, tool_name: String, input: String, executed, dynamic, meta| {
        GenerateContent::ToolCall(RawToolCall {
            tool_call_id: id.to_string(),
            tool_name,
            input,
            provider_executed: executed,
            dynamic,
            provider_metadata: meta,
        })
    };
    let result = |id: &str, tool_name: String, value: Value, meta| {
        GenerateContent::ToolResult(ToolResult {
            tool_call_id: id.to_string(),
            tool_name,
            result: value,
            is_error: None,
            preliminary: None,
            dynamic: None,
            provider_metadata: meta,
        })
    };
    let pair = |canonical: &str, input: String, output: Value| {
        vec![
            call(item_id, name(canonical), input, Some(true), None, None),
            result(item_id, name(canonical), output, None),
        ]
    };
    match kind {
        "image_generation_call" => pair(
            "image_generation",
            "{}".into(),
            json!({"result": part["result"]}),
        ),
        "web_search_call" => pair(
            "web_search",
            "{}".into(),
            web_search_output(&part["action"]),
        ),
        "file_search_call" => pair(
            "file_search",
            "{}".into(),
            json!({
                "queries": part["queries"],
                "results": part["results"].as_array().map(|results| results.iter().map(|value| {
                    json!({"attributes": value["attributes"], "fileId": value["file_id"],
                        "filename": value["filename"], "score": value["score"], "text": value["text"]})
                }).collect::<Vec<_>>()),
            }),
        ),
        "code_interpreter_call" => pair(
            "code_interpreter",
            json!({"code": part["code"], "containerId": part["container_id"]}).to_string(),
            json!({"outputs": part["outputs"]}),
        ),
        "computer_call" if part["call_id"].is_null() => pair(
            "computer_use",
            String::new(),
            json!({"type": "computer_use_tool_result", "status": part["status"]}),
        ),
        "computer_call" => vec![call(
            call_id,
            name("computer"),
            computer_input(part).to_string(),
            None,
            None,
            metadata(),
        )],
        "local_shell_call" => vec![call(
            call_id,
            name("local_shell"),
            json!({"action": local_shell_action(&part["action"])}).to_string(),
            None,
            None,
            metadata(),
        )],
        "shell_call" => vec![call(
            call_id,
            name("shell"),
            json!({"action": {"commands": part["action"]["commands"]}}).to_string(),
            shell_provider_executed.then_some(true),
            None,
            metadata(),
        )],
        "shell_call_output" => vec![result(
            call_id,
            name("shell"),
            json!({"output": part["output"].as_array().map(|items| items.iter().map(|item| {
                let outcome = if item["outcome"]["type"] == "exit" {
                    json!({"type": "exit", "exitCode": item["outcome"]["exit_code"]})
                } else {
                    json!({"type": "timeout"})
                };
                json!({"stdout": item["stdout"], "stderr": item["stderr"], "outcome": outcome})
            }).collect::<Vec<_>>())}),
            None,
        )],
        "apply_patch_call" => vec![call(
            call_id,
            name("apply_patch"),
            json!({"callId": part["call_id"], "operation": apply_patch_operation(&part["operation"])})
                .to_string(),
            None,
            None,
            metadata(),
        )],
        "program" => vec![call(
            call_id,
            name("programmatic_tool_calling"),
            json!({"code": part["code"], "fingerprint": part["fingerprint"]}).to_string(),
            Some(true),
            None,
            metadata(),
        )],
        "program_output" => vec![result(
            call_id,
            name("programmatic_tool_calling"),
            json!({"result": part["result"], "status": part["status"]}),
            metadata(),
        )],
        "tool_search_call" => {
            let mut input = json!({});
            for field in ["arguments", "call_id"] {
                if let Some(value) = part.get(field) {
                    input[field] = value.clone();
                }
            }
            vec![call(
                call_id,
                name("tool_search"),
                input.to_string(),
                (part["execution"] == "server").then_some(true),
                None,
                metadata(),
            )]
        }
        "tool_search_output" => vec![result(
            call_id,
            name("tool_search"),
            json!({"tools": part["tools"]}),
            metadata(),
        )],
        "compaction" => vec![GenerateContent::Custom {
            kind: NamespacedKind::new("openai.compaction")
                .expect("openai.compaction is namespaced"),
            provider_metadata: Some(
                provider_namespace(
                    provider_key,
                    json!({
                        "type": "compaction", "itemId": part["id"],
                        "encryptedContent": part["encrypted_content"],
                    }),
                )
                .expect("provider metadata must be an object"),
            ),
        }],
        "mcp_approval_request" => {
            let tool_call_id = aimux_provider_utils::generate_id();
            let approval_id = part
                .get("approval_request_id")
                .and_then(Value::as_str)
                .unwrap_or(item_id)
                .to_owned();
            vec![
                call(
                    &tool_call_id,
                    format!("mcp.{}", part["name"].as_str().unwrap_or_default()),
                    part["arguments"].as_str().unwrap_or_default().to_owned(),
                    Some(true),
                    Some(true),
                    None,
                ),
                GenerateContent::ToolApprovalRequest(RawToolApprovalRequest {
                    approval_id,
                    tool_call_id,
                    provider_metadata: None,
                }),
            ]
        }
        "mcp_call" => {
            let tool_name = format!("mcp.{}", part["name"].as_str().unwrap_or_default());
            let mut output = json!({"type": "call", "serverLabel": part["server_label"],
                "name": part["name"], "arguments": part["arguments"]});
            for field in ["output", "error"] {
                if let Some(value) = part.get(field).filter(|value| !value.is_null()) {
                    output[field] = value.clone();
                }
            }
            vec![
                call(
                    mcp_tool_call_id.unwrap_or(item_id),
                    tool_name.clone(),
                    part["arguments"].as_str().unwrap_or_default().to_string(),
                    Some(true),
                    Some(true),
                    None,
                ),
                result(
                    mcp_tool_call_id.unwrap_or(item_id),
                    tool_name,
                    output,
                    metadata(),
                ),
            ]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn web_search_output(action: &Value) -> Value {
    match action["type"].as_str() {
        Some("search") => {
            let mut output = json!({"action": {"type": "search"}});
            for field in ["query", "queries"] {
                if let Some(value) = action.get(field).filter(|value| !value.is_null()) {
                    output["action"][field] = value.clone();
                }
            }
            if let Some(sources) = action.get("sources").filter(|value| !value.is_null()) {
                output["sources"] = sources.clone();
            }
            output
        }
        Some("open_page" | "find_in_page") => {
            let kind = if action["type"] == "open_page" {
                "openPage"
            } else {
                "findInPage"
            };
            let mut output = json!({"action": {"type": kind}});
            for field in ["url", "pattern"] {
                if field == "pattern" && kind == "openPage" {
                    continue;
                }
                if let Some(value) = action.get(field) {
                    output["action"][field] = value.clone();
                }
            }
            output
        }
        _ => json!({}),
    }
}

/// A local shell action with its fields in the AI SDK's order: the schema's
/// (`type`, `command`, `timeout_ms`, `user`, `working_directory`, `env`) for a
/// generated call, and the same order under the camel-case names the stream
/// writes (`timeoutMs`, `workingDirectory`).
fn local_shell_action(action: &Value) -> Value {
    let Some(object) = action.as_object() else {
        return action.clone();
    };
    let mut output = serde_json::Map::new();
    for field in [
        "type",
        "command",
        "timeout_ms",
        "timeoutMs",
        "user",
        "working_directory",
        "workingDirectory",
        "env",
    ] {
        if let Some(value) = object.get(field) {
            output.insert(field.to_owned(), value.clone());
        }
    }
    Value::Object(output)
}

/// An apply patch operation with the fields of its type in the order of the
/// AI SDK's schema (`type`, `path`, then `diff`), as zod's parse leaves it.
pub(crate) fn apply_patch_operation(operation: &Value) -> Value {
    let fields: &[&str] = match operation["type"].as_str() {
        Some("create_file" | "update_file") => &["path", "diff"],
        Some("delete_file") => &["path"],
        _ => return operation.clone(),
    };
    let mut output = json!({"type": operation["type"]});
    for field in fields {
        if let Some(value) = operation.get(*field) {
            output[*field] = value.clone();
        }
    }
    output
}

/// Keeps the fields each computer action type has, renaming the scroll
/// offsets and leaving out a null `keys`, as the AI SDK's `mapComputerAction`.
fn computer_action(action: &Value) -> Value {
    let fields: &[(&str, &str)] = match action["type"].as_str() {
        Some("click") => &[
            ("button", "button"),
            ("x", "x"),
            ("y", "y"),
            ("keys", "keys"),
        ],
        Some("double_click" | "move") => &[("x", "x"), ("y", "y"), ("keys", "keys")],
        Some("drag") => &[("path", "path"), ("keys", "keys")],
        Some("keypress") => &[("keys", "keys")],
        Some("scroll") => &[
            ("x", "x"),
            ("y", "y"),
            ("scroll_x", "scrollX"),
            ("scroll_y", "scrollY"),
            ("keys", "keys"),
        ],
        Some("type") => &[("text", "text")],
        Some("screenshot" | "wait") => &[],
        _ => return action.clone(),
    };
    let mut output = json!({"type": action["type"]});
    for (from, to) in fields {
        if let Some(value) = action.get(*from).filter(|value| !value.is_null()) {
            output[*to] = value.clone();
        }
    }
    output
}

fn computer_input(part: &Value) -> Value {
    let actions = part["actions"].as_array().cloned().unwrap_or_else(|| {
        part.get("action")
            .filter(|action| !action.is_null())
            .cloned()
            .into_iter()
            .collect()
    });
    let actions: Vec<_> = actions.iter().map(computer_action).collect();
    let checks: Vec<_> = part["pending_safety_checks"]
        .as_array()
        .map(|checks| {
            checks
                .iter()
                .map(|check| {
                    let mut result = json!({"id": check["id"]});
                    for field in ["code", "message"] {
                        if let Some(value) = check.get(field).filter(|value| !value.is_null()) {
                            result[field] = value.clone();
                        }
                    }
                    result
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"actions": actions, "pendingSafetyChecks": checks, "status": part["status"]})
}
