//! Hugging Face Responses API language model.
//!
//! Implements the [`LanguageModel`] trait against the Hugging Face Responses
//! API (`POST /responses`). This is the lightest Responses implementation in
//! the SDK: it supports function tools only (no built-in tools), uses
//! `text.format` for structured output, and surfaces reasoning content from
//! `reasoning` output items.
//!
//! Translation of the TS sources:
//! - `huggingface-responses-language-model.ts` — request/response + streaming
//! - `convert-to-huggingface-responses-messages.ts` — prompt → `input` array
//! - `huggingface-responses-prepare-tools.ts` — tool / tool-choice mapping
//! - `convert-huggingface-responses-usage.ts` — usage mapping
//! - `map-huggingface-responses-finish-reason.ts` — finish-reason mapping
//! - `huggingface-responses-language-model-options.ts` — provider options

use std::collections::HashMap;

use async_trait::async_trait;
use base64::Engine;
use futures::StreamExt;
use serde_json::{Map, Value, json};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::tool::Tool;
use aimux_core::types::{
    FinishReason, FinishReasonUnified, ResponseMetadata, TokenUsage, Usage, Warning,
};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, send, send_stream};
use aimux_stream::SseStream;

use super::HuggingFaceConfig;

const PROVIDER_NAME: &str = "huggingface";

// ─────────────────────────────────────────────────────────────────────────────
// Model
// ─────────────────────────────────────────────────────────────────────────────

/// A Hugging Face Responses API language model.
///
/// Created via [`super::HuggingFaceProvider::responses_model`].
pub struct HuggingFaceResponsesModel {
    model_id: String,
    config: HuggingFaceConfig,
}

impl HuggingFaceResponsesModel {
    pub fn new(model_id: String, config: HuggingFaceConfig) -> Self {
        Self { model_id, config }
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.config.0.base_url)
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.0.api_key),
        );
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }
}

/// Build the header list for a JSON POST: auth/extra headers + `Content-Type`.
///
/// Returns a `Vec<(String, String)>` for `HttpRequest` — no reqwest types.
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> =
        headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

#[async_trait]
impl LanguageModel for HuggingFaceResponsesModel {
    fn provider(&self) -> &str {
        PROVIDER_NAME
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let request = build_request_body_with_warnings(&self.model_id, options, false)?;
        let body = request.body;
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
            },
            self.config.0.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;

        let response: Value =
            serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;

        // Check for an error field in the response body.
        if let Some(error) = response.get("error")
            && !error.is_null()
        {
            let message = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(AiMuxError::ApiCall(message.to_string()));
        }

        let content = build_generate_content(&response);
        let finish_reason = parse_finish_reason(&response);
        let usage = convert_usage(response.get("usage"));

        let response_id = response
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let created_at = response
            .get("created_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let model = response
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(GenerateResult {
            content,
            finish_reason,
            usage,
            warnings: request.warnings,
            provider_metadata: Some(json!({
                "huggingface": { "responseId": response_id }
            })),
            response: ResponseMetadata {
                id: response_id,
                timestamp: format_timestamp(created_at),
                model_id: model,
            },
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let request = build_request_body_with_warnings(&self.model_id, options, true)?;
        let body = request.body;
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send_stream(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.endpoint(),
                headers: build_header_list(&headers),
                body: HttpBody::Json(body.clone()),
            },
            self.config.0.retry_config,
            &DEFAULT_ERROR_STRUCTURE,
        )
        .await?;

        let response_headers = resp.headers;
        let sse_stream = SseStream::new(resp.body);

        let warnings = request.warnings.clone();
        let stream = async_stream::stream! {
            // First part: StreamStart.
            yield Ok(StreamPart::StreamStart { warnings });

            let mut finish_reason = FinishReason {
                unified: FinishReasonUnified::Other,
                raw: None,
            };
            let mut response_id: Option<String> = None;
            let mut usage_raw: Option<Value> = None;
            let mut stream_errored = false;

            let mut event_iter = sse_stream;

            while let Some(event) = event_iter.next().await {
                if stream_errored {
                    break;
                }

                match event {
                    Ok(sse_event) => {
                        let parsed: Value = match serde_json::from_str(&sse_event.data) {
                            Ok(v) => v,
                            Err(e) => {
                                finish_reason = FinishReason {
                                    unified: FinishReasonUnified::Error,
                                    raw: None,
                                };
                                yield Ok(StreamPart::Error {
                                    error: AiMuxError::Json(e.to_string()),
                                });
                                stream_errored = true;
                                continue;
                            }
                        };

                        let chunk_type = parsed
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        match chunk_type {
                            "response.created" => {
                                if let Some(resp) = parsed.get("response") {
                                    response_id = resp
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    let created_at = resp
                                        .get("created_at")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0);
                                    let model = resp
                                        .get("model")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());
                                    yield Ok(StreamPart::ResponseMetadata {
                                        id: response_id.clone(),
                                        timestamp: format_timestamp(created_at),
                                        model_id: model,
                                    });
                                }
                            }

                            "response.output_item.added" => {
                                if let Some(item) = parsed.get("item") {
                                    let item_type = item
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    match item_type {
                                        "message" => {
                                            let role = item
                                                .get("role")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if role == "assistant" {
                                                let id = item
                                                    .get("id")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                yield Ok(StreamPart::TextStart { id });
                                            }
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
                                            yield Ok(StreamPart::ToolInputStart {
                                                id: call_id,
                                                tool_name: name,
                                                provider_executed: None,
                                                dynamic: None,
                                            });
                                        }
                                        "reasoning" => {
                                            let id = item
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            yield Ok(StreamPart::ReasoningStart {
                                                id: id.clone(),
                                                provider_metadata: Some(json!({
                                                    "huggingface": { "itemId": id }
                                                })),
                                            });
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            "response.output_item.done" => {
                                if let Some(item) = parsed.get("item") {
                                    let item_type = item
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    match item_type {
                                        "message" => {
                                            let role = item
                                                .get("role")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");
                                            if role == "assistant" {
                                                let id = item
                                                    .get("id")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                yield Ok(StreamPart::TextEnd { id });
                                            }
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
                                            let arguments = item
                                                .get("arguments")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("{}");
                                            let input: Value = serde_json::from_str(arguments)
                                                .unwrap_or_else(|_| {
                                                    Value::String(arguments.to_string())
                                                });

                                            yield Ok(StreamPart::ToolInputEnd {
                                                id: call_id.clone(),
                                            });
                                            yield Ok(StreamPart::ToolCall {
                                                tool_call_id: call_id.clone(),
                                                tool_name: name.clone(),
                                                input,
                                                provider_executed: None,
                                                dynamic: None,
                                            });

                                            if let Some(output) =
                                                item.get("output").and_then(|v| v.as_str())
                                            {
                                                yield Ok(StreamPart::ToolResult {
                                                    tool_call_id: call_id,
                                                    tool_name: name.clone(),
                                                    result: Value::String(output.to_string()),
                                                    is_error: None,
                                                    preliminary: None,
                                                    dynamic: None,
                                                    provider_metadata: None,
                                                });
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            "response.output_text.delta" => {
                                let item_id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let delta = parsed
                                    .get("delta")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                yield Ok(StreamPart::TextDelta { id: item_id, delta });
                            }

                            "response.reasoning_text.delta" => {
                                let item_id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let delta = parsed
                                    .get("delta")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                yield Ok(StreamPart::ReasoningDelta {
                                    id: item_id,
                                    delta,
                provider_metadata: None,
            });
                            }

                            "response.reasoning_text.done" => {
                                let item_id = parsed
                                    .get("item_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                yield Ok(StreamPart::ReasoningEnd { id: item_id,
                provider_metadata: None,
            });
                            }

                            "response.completed" => {
                                if let Some(resp) = parsed.get("response") {
                                    response_id = resp
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string());

                                    let incomplete_reason = resp
                                        .get("incomplete_details")
                                        .and_then(|d| d.get("reason"))
                                        .and_then(|r| r.as_str());

                                    finish_reason = FinishReason {
                                        unified: map_finish_reason(
                                            incomplete_reason.unwrap_or("stop"),
                                        ),
                                        raw: incomplete_reason.map(|s| s.to_string()),
                                    };

                                    if let Some(usage) = resp.get("usage")
                                        && !usage.is_null()
                                    {
                                        usage_raw = Some(usage.clone());
                                    }
                                }
                            }

                            _ => {
                                // Unknown chunk type — ignore (matches the TS
                                // fallback schema which silently drops unknowns).
                            }
                        }
                    }
                    Err(e) => {
                        finish_reason = FinishReason {
                            unified: FinishReasonUnified::Error,
                            raw: None,
                        };
                        yield Ok(StreamPart::Error {
                            error: AiMuxError::Stream(e.to_string()),
                        });
                        stream_errored = true;
                    }
                }
            }

            let usage = convert_usage(usage_raw.as_ref());

            yield Ok(StreamPart::Finish {
                finish_reason,
                usage,
                provider_metadata: Some(json!({
                    "huggingface": { "responseId": response_id }
                })),
            });
        };

        Ok(StreamResult {
            stream: Box::pin(stream),
            request_body: Some(body),
            response_headers: Some(response_headers),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Request body construction
// ─────────────────────────────────────────────────────────────────────────────

/// Result of building a request body (body + accumulated warnings).
pub struct RequestBodyResult {
    pub body: Value,
    pub warnings: Vec<Warning>,
}

/// Build the Hugging Face Responses request body from [`CallOptions`].
///
/// Mirrors the TS `getArgs` + `doGenerate`/`doStream` body assembly. The
/// `stream` flag controls the `stream` field in the body.
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
) -> Result<RequestBodyResult, AiMuxError> {
    let mut warnings = Vec::new();

    // Unsupported settings → warnings.
    if options.top_k.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "topK".into(),
            details: None,
        });
    }
    if options.seed.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "seed".into(),
            details: None,
        });
    }
    if options.presence_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "presencePenalty".into(),
            details: None,
        });
    }
    if options.frequency_penalty.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "frequencyPenalty".into(),
            details: None,
        });
    }
    if options.stop_sequences.is_some() {
        warnings.push(Warning::Unsupported {
            feature: "stopSequences".into(),
            details: None,
        });
    }

    let (input, message_warnings) = convert_to_huggingface_responses_messages(&options.prompt)?;
    warnings.extend(message_warnings);

    // Parse provider options (providerOptions.huggingface.*).
    let hf_options = options
        .provider_options
        .as_ref()
        .and_then(|m| m.get("huggingface"));
    let metadata = hf_options.and_then(|o| o.get("metadata")).cloned();
    let instructions = hf_options
        .and_then(|o| o.get("instructions"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let strict_json_schema = hf_options
        .and_then(|o| o.get("strictJsonSchema"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reasoning_effort = hf_options
        .and_then(|o| o.get("reasoningEffort"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Prepare tools.
    let (tools, tool_choice, tool_warnings) =
        prepare_responses_tools(&options.tools, &options.tool_choice);
    warnings.extend(tool_warnings);

    // Assemble the body. Key insertion order follows the TS `baseArgs` object
    // literal so snapshots / strict comparisons match (serde_json preserves
    // insertion order via the `preserve_order` feature).
    let mut body = Map::new();
    body.insert("model".into(), json!(model_id));
    body.insert("input".into(), input);

    if let Some(t) = options.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if let Some(tp) = options.top_p {
        body.insert("top_p".into(), json!(tp));
    }
    if let Some(max) = options.max_output_tokens {
        body.insert("max_output_tokens".into(), json!(max));
    }

    // Structured output (text.format).
    if let Some(ResponseFormat::Json {
        schema,
        name,
        description,
    }) = &options.response_format
        && let Some(schema) = schema
    {
        let mut format = Map::new();
        format.insert("type".into(), json!("json_schema"));
        format.insert("strict".into(), json!(strict_json_schema));
        format.insert(
            "name".into(),
            json!(name.clone().unwrap_or_else(|| "response".to_string())),
        );
        if let Some(desc) = description {
            format.insert("description".into(), json!(desc));
        }
        format.insert("schema".into(), schema.clone());
        body.insert("text".into(), json!({ "format": Value::Object(format) }));
    }

    if let Some(metadata) = metadata {
        body.insert("metadata".into(), metadata);
    }
    if let Some(instructions) = instructions {
        body.insert("instructions".into(), json!(instructions));
    }

    if let Some(tools) = tools {
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(tool_choice) = tool_choice {
        body.insert("tool_choice".into(), tool_choice);
    }

    if let Some(effort) = reasoning_effort {
        body.insert("reasoning".into(), json!({ "effort": effort }));
    }

    body.insert("stream".into(), json!(stream));

    Ok(RequestBodyResult {
        body: Value::Object(body),
        warnings,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Message conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a [`LanguageModelPrompt`] into the Hugging Face Responses `input`
/// array, returning the JSON value and any warnings.
///
/// Mirrors the TS `convertToHuggingFaceResponsesMessages`. Returns an error
/// for unsupported file part variants (provider references, non-image media
/// types) — matching the TS `UnsupportedFunctionalityError`.
pub fn convert_to_huggingface_responses_messages(
    prompt: &LanguageModelPrompt,
) -> Result<(Value, Vec<Warning>), AiMuxError> {
    let mut messages: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for msg in prompt {
        match msg.role {
            Role::System => {
                // System content is a plain string (concatenation of text parts).
                let content: String = msg
                    .content
                    .iter()
                    .filter_map(|p| match p {
                        ContentPart::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                messages.push(json!({ "role": "system", "content": content }));
            }

            Role::User => {
                let mut parts: Vec<Value> = Vec::new();
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } => {
                            parts.push(json!({ "type": "input_text", "text": text }));
                        }
                        ContentPart::FileBase64 {
                            data, media_type, ..
                        } => {
                            parts.push(convert_file_part_base64(media_type, data)?);
                        }
                        ContentPart::FileUrl {
                            url, media_type, ..
                        } => {
                            parts.push(convert_file_part_url(media_type, url)?);
                        }
                        ContentPart::FileReference { .. } => {
                            return Err(AiMuxError::Unsupported(
                                "file parts with provider references".into(),
                            ));
                        }
                        ContentPart::Image {
                            image, media_type, ..
                        } => {
                            let encoded = base64::engine::general_purpose::STANDARD.encode(image);
                            parts.push(convert_file_part_base64(media_type, &encoded)?);
                        }
                        ContentPart::File {
                            data, media_type, ..
                        } => {
                            let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                            parts.push(convert_file_part_base64(media_type, &encoded)?);
                        }
                        _ => {
                            // Other part types (tool-call, tool-result, reasoning)
                            // are not expected in user messages — skip.
                        }
                    }
                }
                messages.push(json!({ "role": "user", "content": parts }));
            }

            Role::Assistant => {
                for part in &msg.content {
                    match part {
                        ContentPart::Text { text, .. } => {
                            messages.push(json!({
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }]
                            }));
                        }
                        ContentPart::Reasoning { text, .. } => {
                            messages.push(json!({
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }]
                            }));
                        }
                        // Tool calls and tool results are handled by the
                        // Responses API — skip (no warning, matching TS).
                        ContentPart::ToolCall { .. } | ContentPart::ToolResult { .. } => {}
                        _ => {}
                    }
                }
            }

            Role::Tool => {
                warnings.push(Warning::Unsupported {
                    feature: "tool messages".into(),
                    details: None,
                });
            }
        }
    }

    Ok((Value::Array(messages), warnings))
}

/// Convert a file part with inline base64 data to the HF `input_image` format.
///
/// Returns the JSON object `{ type: "input_image", image_url: "data:...;base64,..." }`
/// or an error if the media type is not an image.
fn convert_file_part_base64(media_type: &str, data: &str) -> Result<Value, AiMuxError> {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" {
        let full_type = resolve_full_media_type_base64(media_type, data)?;
        Ok(json!({
            "type": "input_image",
            "image_url": format!("data:{};base64,{}", full_type, data)
        }))
    } else {
        Err(AiMuxError::Unsupported(format!(
            "file part media type {}",
            media_type
        )))
    }
}

/// Convert a file part with a URL source to the HF `input_image` format.
fn convert_file_part_url(media_type: &str, url: &str) -> Result<Value, AiMuxError> {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" {
        Ok(json!({
            "type": "input_image",
            "image_url": url
        }))
    } else {
        Err(AiMuxError::Unsupported(format!(
            "file part media type {}",
            media_type
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Media type detection (top-level-only media type resolution)
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the top-level segment of a media type (the portion before `/`).
///
/// `"image/png"` → `"image"`, `"image/*"` → `"image"`, `"image"` → `"image"`.
fn get_top_level_media_type(media_type: &str) -> &str {
    match media_type.find('/') {
        Some(idx) => &media_type[..idx],
        None => media_type,
    }
}

/// Returns `true` only when the media type has a non-empty, non-wildcard
/// subtype (i.e. matches `type/subtype` where `subtype` is not `*`).
fn is_full_media_type(media_type: &str) -> bool {
    match media_type.find('/') {
        Some(idx) => {
            let subtype = &media_type[idx + 1..];
            !subtype.is_empty() && subtype != "*"
        }
        None => false,
    }
}

/// Resolve a media type to its full `type/subtype` form.
///
/// - If already a full media type, return as-is.
/// - Otherwise, detect the subtype from the base64 data bytes.
fn resolve_full_media_type_base64(media_type: &str, data: &str) -> Result<String, AiMuxError> {
    if is_full_media_type(media_type) {
        return Ok(media_type.to_string());
    }

    let top_level = get_top_level_media_type(media_type);
    if let Some(detected) = detect_media_type_from_base64(data, top_level) {
        return Ok(detected);
    }

    Err(AiMuxError::Unsupported(format!(
        "file of media type \"{}\" must specify subtype since it could not be auto-detected",
        media_type
    )))
}

/// Image media type signatures (magic-byte prefixes).
///
/// `None` in the prefix means "any byte" (wildcard), matching the TS signature
/// table.
const IMAGE_SIGNATURES: &[(&str, &[Option<u8>])] = &[
    ("image/gif", &[Some(0x47), Some(0x49), Some(0x46)]),
    (
        "image/png",
        &[Some(0x89), Some(0x50), Some(0x4E), Some(0x47)],
    ),
    ("image/jpeg", &[Some(0xFF), Some(0xD8)]),
    (
        "image/webp",
        &[
            Some(0x52),
            Some(0x49),
            Some(0x46),
            Some(0x46), // RIFF
            None,
            None,
            None,
            None, // file size (variable)
            Some(0x57),
            Some(0x45),
            Some(0x42),
            Some(0x50), // WEBP
        ],
    ),
    ("image/bmp", &[Some(0x42), Some(0x4D)]),
    (
        "image/tiff",
        &[Some(0x49), Some(0x49), Some(0x2A), Some(0x00)],
    ),
    (
        "image/tiff",
        &[Some(0x4D), Some(0x4D), Some(0x00), Some(0x2A)],
    ),
    (
        "image/avif",
        &[
            Some(0x00),
            Some(0x00),
            Some(0x00),
            Some(0x20),
            Some(0x66),
            Some(0x74),
            Some(0x79),
            Some(0x70),
            Some(0x61),
            Some(0x76),
            Some(0x69),
            Some(0x66),
        ],
    ),
    (
        "image/heic",
        &[
            Some(0x00),
            Some(0x00),
            Some(0x00),
            Some(0x20),
            Some(0x66),
            Some(0x74),
            Some(0x79),
            Some(0x70),
            Some(0x68),
            Some(0x65),
            Some(0x69),
            Some(0x63),
        ],
    ),
];

/// Detect the full media type from base64-encoded data, considering only
/// signatures for the given top-level type.
fn detect_media_type_from_base64(data: &str, top_level: &str) -> Option<String> {
    let signatures = match top_level {
        "image" => IMAGE_SIGNATURES,
        _ => return None,
    };

    // Decode enough bytes to cover the longest signature (12 bytes).
    // 4 base64 chars → 3 bytes; ceil(12 / 3) * 4 = 16 chars.
    let max_chars = 16.min(data.len());
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data[..max_chars])
        .ok()?;

    for (media_type, prefix) in signatures {
        if bytes.len() >= prefix.len()
            && prefix
                .iter()
                .enumerate()
                .all(|(i, &b)| b.is_none() || bytes[i] == b.unwrap())
        {
            return Some(media_type.to_string());
        }
    }

    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool preparation
// ─────────────────────────────────────────────────────────────────────────────

/// Prepare tools and tool choice for the Hugging Face Responses API.
///
/// Returns `(tools, tool_choice, warnings)`. Only function tools are supported;
/// provider-defined tools produce a warning.
pub fn prepare_responses_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: &ToolChoice,
) -> (Option<Vec<Value>>, Option<Value>, Vec<Warning>) {
    let tools = match tools {
        Some(t) if !t.is_empty() => t,
        _ => return (None, None, vec![]),
    };

    let mut hf_tools: Vec<Value> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();

    for tool in tools {
        match tool {
            Tool::Function(ft) => {
                let mut t = Map::new();
                t.insert("type".into(), json!("function"));
                t.insert("name".into(), json!(ft.name));
                if let Some(desc) = &ft.description {
                    t.insert("description".into(), json!(desc));
                }
                t.insert("parameters".into(), ft.input_schema.clone());
                hf_tools.push(Value::Object(t));
            }
            Tool::Provider(pt) => {
                warnings.push(Warning::Unsupported {
                    feature: format!("provider-defined tool {}", pt.id),
                    details: None,
                });
            }
        }
    }

    let mapped_tool_choice = match tool_choice {
        ToolChoice::Auto => Some(json!("auto")),
        ToolChoice::Required => Some(json!("required")),
        ToolChoice::None => None, // not supported, ignore
        ToolChoice::Tool { tool_name } => Some(json!({
            "type": "function",
            "function": { "name": tool_name }
        })),
    };

    (Some(hf_tools), mapped_tool_choice, warnings)
}

// ─────────────────────────────────────────────────────────────────────────────
// Response parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Build the `GenerateContent` vector from a Responses API JSON body.
///
/// Mirrors the TS `doGenerate` output-array processing.
fn build_generate_content(response: &Value) -> Vec<GenerateContent> {
    let mut content = Vec::new();
    let mut source_id_counter = 0u32;

    let empty_vec: Vec<Value> = Vec::new();
    let output = response
        .get("output")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_vec);

    for part in output {
        let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match part_type {
            "message" => {
                let content_parts = part
                    .get("content")
                    .and_then(|v| v.as_array())
                    .unwrap_or(&empty_vec);
                for cp in content_parts {
                    let text = cp.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    content.push(GenerateContent::Text {
                        text: text.to_string(),
                    });

                    // Process annotations → source parts.
                    if let Some(annotations) = cp.get("annotations").and_then(|v| v.as_array()) {
                        for ann in annotations {
                            let url = ann.get("url").and_then(|v| v.as_str()).unwrap_or("");
                            let title = ann.get("title").and_then(|v| v.as_str());
                            content.push(GenerateContent::Source {
                                id: format!("id-{}", source_id_counter),
                                source_type: "url".to_string(),
                                url: Some(url.to_string()),
                                title: title.map(|s| s.to_string()),
                            });
                            source_id_counter += 1;
                        }
                    }
                }
            }

            "reasoning" => {
                let content_parts = part
                    .get("content")
                    .and_then(|v| v.as_array())
                    .unwrap_or(&empty_vec);
                for cp in content_parts {
                    let text = cp.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let item_id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    content.push(GenerateContent::Reasoning {
                        text: text.to_string(),
                        provider_metadata: Some(json!({
                            "huggingface": { "itemId": item_id }
                        })),
                    });
                }
            }

            "function_call" => {
                let call_id = part.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let name = part.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = part
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(arguments)
                    .unwrap_or_else(|_| Value::String(arguments.to_string()));
                content.push(GenerateContent::ToolCall {
                    tool_call_id: call_id.to_string(),
                    tool_name: name.to_string(),
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                });
                // Tool result — the Rust GenerateContent enum has no ToolResult
                // variant, so it is omitted (the TS emits it as a separate
                // content item).
            }

            "mcp_call" => {
                let id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = part.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = part
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let input: Value = serde_json::from_str(arguments)
                    .unwrap_or_else(|_| Value::String(arguments.to_string()));
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.to_string(),
                    tool_name: name.to_string(),
                    input,
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }

            "mcp_list_tools" => {
                let id = part.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let server_label = part.get("server_label").cloned().unwrap_or(Value::Null);
                content.push(GenerateContent::ToolCall {
                    tool_call_id: id.to_string(),
                    tool_name: "list_tools".to_string(),
                    input: json!({ "server_label": server_label }),
                    provider_executed: None,
                    dynamic: None,
                    provider_metadata: None,
                });
            }

            _ => {}
        }
    }

    content
}

/// Parse the finish reason from a Responses API JSON body.
fn parse_finish_reason(response: &Value) -> FinishReason {
    let incomplete_reason = response
        .get("incomplete_details")
        .and_then(|d| d.get("reason"))
        .and_then(|r| r.as_str());

    FinishReason {
        unified: map_finish_reason(incomplete_reason.unwrap_or("stop")),
        raw: incomplete_reason.map(|s| s.to_string()),
    }
}

/// Map a Hugging Face finish-reason string to the unified enum.
fn map_finish_reason(reason: &str) -> FinishReasonUnified {
    match reason {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        "content_filter" => FinishReasonUnified::ContentFilter,
        "tool_calls" => FinishReasonUnified::ToolCalls,
        "error" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Usage conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a Hugging Face Responses usage object to the unified [`Usage`].
///
/// Mirrors the TS `convertHuggingFaceResponsesUsage`. `None` / null usage
/// produces an all-`None` `Usage` (matching the TS undefined-fields case).
fn convert_usage(usage: Option<&Value>) -> Usage {
    let u = match usage {
        None | Some(Value::Null) => return Usage::default(),
        Some(u) => u,
    };

    let input_tokens = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let output_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let cached_tokens = u
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let reasoning_tokens = u
        .get("output_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    Usage {
        input_tokens: TokenUsage {
            total: Some(input_tokens),
            no_cache: Some(input_tokens - cached_tokens),
            cache_read: Some(cached_tokens),
            cache_write: None,
            ..Default::default()
        },
        output_tokens: TokenUsage {
            total: Some(output_tokens),
            text: Some(output_tokens - reasoning_tokens),
            reasoning: Some(reasoning_tokens),
            ..Default::default()
        },
        raw: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Format a Unix timestamp (seconds) as an ISO-8601 string with millisecond
/// precision and a `Z` suffix, matching the TS `new Date(ts * 1000)` snapshot
/// format (e.g. `"2025-03-06T13:50:19.000Z"`).
fn format_timestamp(secs: u64) -> Option<String> {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(secs as i64, 0)?;
    Some(dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}
