//! OpenAI language model — implements `LanguageModel` trait.
//!
//! The HTTP request/response handling lives in the free functions
//! [`execute_generate`] and [`execute_stream`], which take an endpoint URL, a
//! header map and a model id. They call `http::send` / `http::send_stream` —
//! **no `reqwest` types cross this boundary**. This lets other providers that
//! speak the OpenAI chat-completions wire format (notably Azure OpenAI) reuse
//! the conversion + streaming logic while supplying their own URL and auth.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
use aimux_core::stream_part::StreamPart;
use aimux_core::types::{FinishReason, FinishReasonUnified, ResponseMetadata, Usage};

use aimux_provider_utils::response::DEFAULT_ERROR_STRUCTURE;
use aimux_provider_utils::{
    HttpBody, HttpMethod, HttpRequest, RetryConfig, send_stream_timed, send_timed,
};
use aimux_stream::SseStream;

use super::OpenAIConfig;
use super::convert::{
    RequestBodyResult, build_request_body_with_warnings_fallible, parse_finish_reason,
};
use super::types::{ChatCompletionResponse, StreamChunk, UsageResponse};

/// An OpenAI-compatible language model.
///
/// Does **not** hold an HTTP client — `http::send` / `http::send_stream` use the
/// process-wide shared `Client` internally (RFC-0009 §4.1).
pub struct OpenAIModel {
    model_id: String,
    config: OpenAIConfig,
}

impl OpenAIModel {
    pub fn new(model_id: String, config: OpenAIConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key),
        );
        if let Some(ref org) = self.config.org_id {
            headers.insert("OpenAI-Organization".to_string(), org.clone());
        }
        if let Some(ref project) = self.config.project {
            headers.insert("OpenAI-Project".to_string(), project.clone());
        }
        // Provider-level headers (from OpenAIConfig.with_headers).
        if let Some(ref config_headers) = self.config.headers {
            for (k, v) in config_headers {
                headers.insert(k.clone(), v.clone());
            }
        }
        // Per-call headers (from CallOptions.headers), overriding provider-level.
        if let Some(extra) = extra {
            for (k, v) in extra {
                headers.insert(k.clone(), v.clone());
            }
        }
        headers
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.config.base_url)
    }
}

/// Resolve the effective `RetryConfig`: if the caller passed a per-call
/// `max_retries` override, clone the provider config and substitute it;
/// otherwise return the provider config as-is (RFC-0017).
fn resolve_retry_config(provider: &RetryConfig, max_retries_override: Option<u32>) -> RetryConfig {
    match max_retries_override {
        Some(n) => RetryConfig {
            max_retries: n,
            ..*provider
        },
        None => *provider,
    }
}

/// Merge provider-level `body_overrides` into the per-call options (RFC-0017).
///
/// Provider-level overrides are applied first (lower priority); per-call
/// `body_overrides` from `CallOptions` are merged on top. If neither is
/// present, the options are returned unchanged (cheap clone).
fn merge_body_overrides(options: &CallOptions, provider_overrides: &Option<Value>) -> CallOptions {
    match (provider_overrides, &options.body_overrides) {
        (Some(provider), Some(call)) => {
            // Merge: provider first, then call (call wins).
            let mut merged = provider.clone();
            crate::openai::convert::deep_merge_json(&mut merged, call);
            let mut opts = options.clone();
            opts.body_overrides = Some(merged);
            opts
        }
        (Some(provider), None) => {
            let mut opts = options.clone();
            opts.body_overrides = Some(provider.clone());
            opts
        }
        (None, _) => options.clone(),
    }
}

// ── Usage conversion ─────────────────────────────────────────────────────────

/// Convert an OpenAI `UsageResponse` into the core `Usage` type.
///
/// Mirrors the TS `convertOpenAIChatUsage`:
/// - `input.total = prompt_tokens`
/// - `input.noCache = prompt_tokens - cached_tokens - cache_write_tokens`
/// - `input.cacheRead = cached_tokens`
/// - `input.cacheWrite = cache_write_tokens`
///
/// `usage_raw` is the provider's original `usage` JSON object, preserved
/// verbatim in `Usage.raw` (M10, RFC-0016). Vendor-specific fields not part
/// of `UsageResponse` (e.g. DeepSeek `prompt_cache_hit_tokens`) survive only
/// through this raw value.
fn convert_usage(usage: &UsageResponse, usage_raw: Option<&Value>) -> Usage {
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    let completion_tokens = usage.completion_tokens.unwrap_or(0);

    // Cache-read tokens: prefer the top-level `cached_tokens` (Moonshot format)
    // over the nested `prompt_tokens_details.cached_tokens` (OpenAI format).
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

    // Use saturating subtraction: some OpenAI-compatible servers (e.g. vLLM
    // serving Qwen3 reasoning models via Doubleword) report
    // `reasoning_tokens > completion_tokens`, and cached + cache-write tokens
    // can exceed `prompt_tokens`. Saturating to 0 avoids a panic on these
    // real-world responses; the non-underflow path is unchanged.
    let no_cache = prompt_tokens
        .saturating_sub(cached)
        .saturating_sub(cache_write.unwrap_or(0));

    // Reasoning tokens from completion_tokens_details.
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);
    let text_tokens = completion_tokens.saturating_sub(reasoning_tokens);

    Usage {
        input_tokens: aimux_core::types::TokenUsage {
            total: Some(prompt_tokens),
            no_cache: Some(no_cache),
            cache_read: Some(cached),
            cache_write,
            ..Default::default()
        },
        output_tokens: aimux_core::types::TokenUsage {
            total: Some(completion_tokens),
            text: Some(text_tokens),
            reasoning: Some(reasoning_tokens),
            ..Default::default()
        },
        // M10 (RFC-0016): keep the provider's original usage JSON verbatim —
        // vendor-specific fields (e.g. Moonshot `cached_tokens`, DeepSeek
        // `prompt_cache_hit_tokens`) are otherwise lost for audit/billing.
        raw: usage_raw.cloned(),
    }
}

// ── Tool-call accumulator (streaming) ────────────────────────────────────────

/// Accumulates a streamed tool call's id, name, and argument fragments.
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

#[async_trait]
impl LanguageModel for OpenAIModel {
    fn provider(&self) -> &str {
        "openai"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let retry_config = resolve_retry_config(&self.config.retry_config, options.max_retries);
        let options = merge_body_overrides(options, &self.config.body_overrides);
        execute_generate(
            &self.endpoint(),
            &headers,
            &self.model_id,
            &options,
            &self.config.provider,
            &self.config.profile,
            &retry_config,
        )
        .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let headers = self.build_headers(options.headers.as_ref());
        let retry_config = resolve_retry_config(&self.config.retry_config, options.max_retries);
        let options = merge_body_overrides(options, &self.config.body_overrides);
        execute_stream(
            &self.endpoint(),
            &headers,
            &self.model_id,
            &options,
            &self.config.provider,
            &self.config.profile,
            &retry_config,
        )
        .await
    }
}

// ── Shared OpenAI chat-completions execution ─────────────────────────────────
//
// These free functions contain the actual HTTP + response-parsing logic. They
// are `pub` so that providers speaking the OpenAI wire format (Azure OpenAI)
// can reuse them with their own endpoint URL and auth headers.

/// Build the header list for a JSON POST: auth/extra headers + `Content-Type`.
///
/// Returns a `Vec<(String, String)>` for `HttpRequest` — no reqwest types.
fn build_header_list(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    list.push(("Content-Type".to_string(), "application/json".to_string()));
    list
}

/// Execute a non-streaming OpenAI chat-completion request.
///
/// `endpoint` is the full chat-completions URL; `headers` carries the auth
/// headers (and any extra/request headers); `model_id` is placed in the
/// request body's `model` field.
pub async fn execute_generate(
    endpoint: &str,
    headers: &HashMap<String, String>,
    model_id: &str,
    options: &CallOptions,
    provider: &str,
    profile: &super::OpenAICompatProfile,
    retry_config: &RetryConfig,
) -> Result<GenerateResult, AiMuxError> {
    let request_result =
        build_request_body_with_warnings_fallible(model_id, options, false, provider, profile)?;
    let body = request_result.body;

    let resp = send_timed(
        HttpRequest {
            method: HttpMethod::Post,
            url: endpoint.to_string(),
            headers: build_header_list(headers),
            body: HttpBody::Json(body.clone()),

            abort_signal: options.abort_signal.clone(),
            call_id: options.call_id.clone(),
            recording_context: options.recording_context.clone(),
        },
        *retry_config,
        &DEFAULT_ERROR_STRUCTURE,
        options.timeout.map(Into::into),
    )
    .await?;

    let response_headers = resp.headers;

    // Parse the raw body once: the `Value` keeps the provider's original
    // fields (incl. vendor-specific usage fields) for `Usage.raw` (M10).
    let response_value: Value =
        serde_json::from_slice(&resp.body).map_err(|e| AiMuxError::Json(e.to_string()))?;
    let data: ChatCompletionResponse = serde_json::from_value(response_value.clone())
        .map_err(|e| AiMuxError::Json(e.to_string()))?;

    let choice = data
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| AiMuxError::Provider("no choices in response".to_string()))?;

    // Build content array.
    let mut content = Vec::new();
    if let Some(text) = choice.message.content
        && !text.is_empty()
    {
        content.push(GenerateContent::Text {
            text,
            provider_metadata: None,
        });
    }
    // Reasoning: prefer reasoning_content over reasoning (DeepSeek/阿里通义).
    let reasoning_text = choice
        .message
        .reasoning_content
        .clone()
        .or_else(|| choice.message.reasoning.clone());
    if let Some(reasoning) = reasoning_text
        && !reasoning.is_empty()
    {
        content.push(GenerateContent::Reasoning {
            text: reasoning,
            provider_metadata: None,
        });
    }
    if let Some(tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let input: Value = serde_json::from_str(&tc.function.arguments)
                .unwrap_or_else(|_| Value::String(tc.function.arguments.clone()));
            content.push(GenerateContent::ToolCall {
                tool_call_id: tc.id,
                tool_name: tc.function.name,
                input,
                provider_executed: None,
                dynamic: None,
                thought_signature: None,
                provider_metadata: None,
            });
        }
    }
    // Parse annotations (URL citations) → Source content items.
    if let Some(annotations) = choice.message.annotations {
        for (i, ann) in annotations.iter().enumerate() {
            if ann.get("type").and_then(|v| v.as_str()) == Some("url_citation")
                && let Some(uc) = ann.get("url_citation")
            {
                content.push(GenerateContent::Source {
                    id: format!("annotation-{}", i),
                    source_type: "url".to_string(),
                    url: uc
                        .get("url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    title: uc
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    provider_metadata: None,
                });
            }
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

    let usage = convert_usage(&data.usage, response_value.get("usage"));

    // Build provider metadata: logprobs + prediction tokens.
    let mut pm_openai = serde_json::json!({});
    if let Some(ref lp) = choice.logprobs
        && let Some(content_lp) = lp.get("content")
    {
        pm_openai["logprobs"] = content_lp.clone();
    }
    // Accepted/rejected prediction tokens from completion_tokens_details.
    if let Some(ref details) = data.usage.completion_tokens_details {
        if let Some(apt) = details.accepted_prediction_tokens {
            pm_openai["acceptedPredictionTokens"] = json!(apt);
        }
        if let Some(rpt) = details.rejected_prediction_tokens {
            pm_openai["rejectedPredictionTokens"] = json!(rpt);
        }
    }
    let provider_metadata = Some(serde_json::json!({ "openai": pm_openai }));

    Ok(GenerateResult {
        content,
        finish_reason,
        usage,
        warnings: request_result.warnings,
        provider_metadata,
        response: ResponseMetadata {
            id: Some(data.id),
            timestamp: None,
            model_id: Some(data.model),
        },
        request_body: Some(body),
        response_headers: Some(response_headers),
    })
}

/// Execute a streaming OpenAI chat-completion request.
///
/// `endpoint` is the full chat-completions URL; `headers` carries the auth
/// headers (and any extra/request headers); `model_id` is placed in the
/// request body's `model` field.
pub async fn execute_stream(
    endpoint: &str,
    headers: &HashMap<String, String>,
    model_id: &str,
    options: &CallOptions,
    provider: &str,
    profile: &super::OpenAICompatProfile,
    retry_config: &RetryConfig,
) -> Result<StreamResult, AiMuxError> {
    let request_result =
        build_request_body_with_warnings_fallible(model_id, options, true, provider, profile)?;
    // M9 (RFC-0016): keep the warnings computed while building the body —
    // they are emitted in `StreamStart` below instead of being dropped.
    let RequestBodyResult { body, warnings } = request_result;

    let resp = send_stream_timed(
        HttpRequest {
            method: HttpMethod::Post,
            url: endpoint.to_string(),
            headers: build_header_list(headers),
            body: HttpBody::Json(body.clone()),

            abort_signal: options.abort_signal.clone(),
            call_id: options.call_id.clone(),
            recording_context: options.recording_context.clone(),
        },
        *retry_config,
        &DEFAULT_ERROR_STRUCTURE,
        options.timeout.map(Into::into),
    )
    .await?;

    let response_headers = resp.headers;

    let mut sse_stream = SseStream::new(resp.body);

    // Peek at the first SSE event to detect early errors (before any output).
    // The TS SDK rejects the doStream promise when the very first chunk is an
    // error. We replicate that by reading one event here.
    let first_event = sse_stream.next().await;
    if let Some(Ok(ref event)) = first_event
        && let Ok(val) = serde_json::from_str::<Value>(&event.data)
        && let Some(err_obj) = val.get("error")
    {
        return Err(stream_error_to_ai_error(err_obj));
    }

    // Capture the provider's stream usage key before entering the async stream
    // block (the borrowed `profile` cannot be moved into the generator).
    // Some(key) → read streaming usage from `chunk[key].usage`;
    // None → read from the top-level `usage`.
    let stream_usage_key = profile.stream_usage_key;

    // M2 (RFC-0016): capture whether raw chunks should be emitted — the
    // borrowed `options` cannot be moved into the generator.
    let emit_raw_chunks = options.include_raw_chunks == Some(true);

    let stream = async_stream::stream! {
        // First part: StreamStart.
        yield Ok(StreamPart::StreamStart { warnings });

        let text_id = 0usize;
        let mut text_started = false;
        let reasoning_id = "reasoning-0".to_string();
        let mut reasoning_started = false;
        let mut final_usage = Usage::default();
        let mut final_usage_raw: Option<UsageResponse> = None;
        let mut final_finish_reason: Option<FinishReason> = None;
        let mut response_metadata_emitted = false;
        let mut final_logprobs: Option<Value> = None;

        // Tool-call accumulators keyed by OpenAI's `index` field.
        let mut tool_calls: HashMap<usize, ToolCallAccumulator> = HashMap::new();
        let mut tool_call_order: Vec<usize> = Vec::new();

        // Process the first event (already peeked) then the rest.
        let mut event_iter =
            futures::stream::iter(first_event.into_iter()).chain(sse_stream);

        let mut stream_errored = false;

        while let Some(event) = event_iter.next().await {
            if stream_errored {
                break;
            }

            match event {
                Ok(sse_event) => {
                    if sse_event.data == "[DONE]" {
                        break;
                    }

                    // Parse as generic Value first to detect errors.
                    let parsed: Value = match serde_json::from_str(&sse_event.data) {
                        Ok(v) => v,
                        Err(e) => {
                            // Unparsable chunk — emit Error, then finish.
                            yield Ok(StreamPart::Error {
                                error: AiMuxError::Json(e.to_string()),
                            });
                            stream_errored = true;
                            break;
                        }
                    };

                    // M2 (RFC-0016): emit the raw provider chunk for debugging
                    // before it is consumed below. JSON payloads only — the
                    // "[DONE]" sentinel is skipped by the early break above.
                    if emit_raw_chunks {
                        yield Ok(StreamPart::Raw {
                            raw_value: parsed.clone(),
                        });
                    }

                    // Check for mid-stream error.
                    if let Some(err_obj) = parsed.get("error") {
                        yield Ok(StreamPart::Error {
                            error: stream_error_to_ai_error(err_obj),
                        });
                        stream_errored = true;
                        break;
                    }

                    // Extract streaming usage based on profile.stream_usage_key
                    // (captured before the stream block). Some(key) reads usage
                    // from the provider-specific sub-object `key` (e.g. Groq's
                    // "x_groq"); None reads the top-level "usage". This is done
                    // from the raw JSON chunk before it is consumed below.
                    // `chunk_usage_raw` keeps the provider's original object for
                    // `Usage.raw` (M10).
                    let chunk_usage_raw: Option<Value> = match stream_usage_key {
                        Some(key) => parsed.get(key).and_then(|v| v.get("usage")).cloned(),
                        None => parsed.get("usage").cloned(),
                    };
                    let chunk_usage: Option<UsageResponse> = chunk_usage_raw
                        .as_ref()
                        .and_then(|u| serde_json::from_value(u.clone()).ok());

                    // Parse as StreamChunk.
                    let chunk: StreamChunk = match serde_json::from_value(parsed) {
                        Ok(c) => c,
                        Err(e) => {
                            yield Ok(StreamPart::Error {
                                error: AiMuxError::Json(e.to_string()),
                            });
                            stream_errored = true;
                            break;
                        }
                    };

                    // Emit ResponseMetadata from the first valid chunk.
                    if !response_metadata_emitted
                        && (chunk.id.is_some() || chunk.model.is_some())
                    {
                        response_metadata_emitted = true;
                        yield Ok(StreamPart::ResponseMetadata {
                            id: chunk.id.clone(),
                            timestamp: None,
                            model_id: chunk.model.clone(),
                        });
                    }

                    // Update usage based on profile.stream_usage_key.
                    if let Some(usage) = &chunk_usage {
                        final_usage = convert_usage(usage, chunk_usage_raw.as_ref());
                        final_usage_raw = Some(usage.clone());
                    }

                    // Process choices.
                    for choice in chunk.choices {
                        // Capture logprobs from the finish_reason chunk.
                        if let Some(lp) = &choice.logprobs
                            && let Some(content) = lp.get("content") {
                                final_logprobs = Some(content.clone());
                            }

                        // Reasoning delta: prefer reasoning_content over reasoning.
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

                        // Text delta.
                        if let Some(content) = choice.delta.content {
                            // End active reasoning block before text starts.
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
                                    id: format!("{}", text_id),
                                    provider_metadata: None,
                                });
                            }
                            yield Ok(StreamPart::TextDelta {
                                id: format!("{}", text_id),
                                delta: content,
                                provider_metadata: None,
                            });
                        }

                        // Tool-call deltas.
                        if let Some(tool_call_deltas) = choice.delta.tool_calls {
                            // End active reasoning block before tool calls start.
                            if reasoning_started {
                                yield Ok(StreamPart::ReasoningEnd {
                                    id: reasoning_id.clone(),
                provider_metadata: None,
            });
                                reasoning_started = false;
                            }
                            for dtc in tool_call_deltas {
                                let idx = dtc.index;
                                let func = dtc.function.unwrap_or_default();

                                // New tool call: has id and/or name.
                                let is_new = !tool_calls.contains_key(&idx);
                                if is_new {
                                    let id = dtc.id.unwrap_or_default();
                                    let name = func.name.unwrap_or_default();
                                    tool_calls.insert(
                                        idx,
                                        ToolCallAccumulator {
                                            id: id.clone(),
                                            name: name.clone(),
                                            arguments: String::new(),
                                        },
                                    );
                                    tool_call_order.push(idx);
                                    yield Ok(StreamPart::ToolInputStart {
                                        id,
                                        tool_name: name,
                                        provider_executed: None,
                                        dynamic: None,
                                        title: None,
                                        provider_metadata: None,
                                    });
                                }

                                // Argument delta.
                                // For new tool calls, skip the delta when
                                // arguments are empty (matches TS — the
                                // initial `""` is not emitted). For
                                // continuation chunks, always emit (even
                                // empty, matching TS).
                                if let Some(args) = func.arguments
                                    && (!is_new || !args.is_empty())
                                        && let Some(acc) = tool_calls.get_mut(&idx) {
                                            acc.arguments.push_str(&args);
                                            yield Ok(StreamPart::ToolInputDelta {
                                                id: acc.id.clone(),
                                                delta: args,
                                                provider_metadata: None,
                                            });
                                        }
                            }
                        }

                        // Finish reason.
                        if let Some(reason) = choice.finish_reason {
                            // Close any open reasoning segment.
                            if reasoning_started {
                                yield Ok(StreamPart::ReasoningEnd {
                                    id: reasoning_id.clone(),
                provider_metadata: None,
            });
                                reasoning_started = false;
                            }

                            // Close any open text segment.
                            if text_started {
                                yield Ok(StreamPart::TextEnd {
                                    id: format!("{}", text_id),
                                    provider_metadata: None,
                                });
                                text_started = false;
                            }

                            // Close any open tool calls.
                            for &idx in &tool_call_order {
                                if let Some(acc) = tool_calls.get(&idx) {
                                    yield Ok(StreamPart::ToolInputEnd {
                                        id: acc.id.clone(),
                                        provider_metadata: None,
                                    });
                                    let args = &acc.arguments;
                                    let input: Value = serde_json::from_str(args)
                                        .unwrap_or_else(|_| Value::String(args.clone()));
                                    yield Ok(StreamPart::ToolCall {
                                        tool_call_id: acc.id.clone(),
                                        tool_name: acc.name.clone(),
                                        input,
                                        provider_executed: None,
                                        dynamic: None,
                                        thought_signature: None,
                                        provider_metadata: None,
                                    });
                                }
                            }
                            tool_calls.clear();
                            tool_call_order.clear();

                            final_finish_reason = Some(parse_finish_reason(&reason));
                        }
                    }
                }
                Err(e) => {
                    yield Ok(StreamPart::Error {
                        error: AiMuxError::Stream(e.to_string()),
                    });
                    stream_errored = true;
                    break;
                }
            }
        }

        // Close any remaining open reasoning segment.
        if reasoning_started {
            yield Ok(StreamPart::ReasoningEnd {
                id: reasoning_id.clone(),
                provider_metadata: None,
            });
        }

        // Close any remaining open text segment.
        if text_started {
            yield Ok(StreamPart::TextEnd {
                id: format!("{}", text_id),
                provider_metadata: None,
            });
        }

        // Close any remaining tool calls (no finish_reason was received).
        for &idx in &tool_call_order {
            if let Some(acc) = tool_calls.get(&idx) {
                yield Ok(StreamPart::ToolInputEnd {
                    id: acc.id.clone(),
                    provider_metadata: None,
                });
                let args = &acc.arguments;
                let input: Value = serde_json::from_str(args)
                    .unwrap_or_else(|_| Value::String(args.clone()));
                yield Ok(StreamPart::ToolCall {
                    tool_call_id: acc.id.clone(),
                    tool_name: acc.name.clone(),
                    input,
                    provider_executed: None,
                    dynamic: None,
                    thought_signature: None,
                    provider_metadata: None,
                });
            }
        }

        // Build provider metadata for the Finish part.
        let mut pm_openai = serde_json::json!({});
        if let Some(ref logprobs) = final_logprobs {
            pm_openai["logprobs"] = json!(logprobs);
        }
        // Prediction tokens from raw usage.
        if let Some(ref raw_usage) = final_usage_raw
            && let Some(ref details) = raw_usage.completion_tokens_details {
                if let Some(apt) = details.accepted_prediction_tokens {
                    pm_openai["acceptedPredictionTokens"] = json!(apt);
                }
                if let Some(rpt) = details.rejected_prediction_tokens {
                    pm_openai["rejectedPredictionTokens"] = json!(rpt);
                }
            }
        let provider_metadata = serde_json::json!({ "openai": pm_openai });

        // Final part: Finish.
        yield Ok(StreamPart::Finish {
            finish_reason: if stream_errored {
                FinishReason {
                    unified: FinishReasonUnified::Error,
                    raw: None,
                }
            } else {
                final_finish_reason.unwrap_or(FinishReason {
                    unified: FinishReasonUnified::Stop,
                    raw: None,
                })
            },
            usage: if stream_errored {
                Usage::default()
            } else {
                final_usage
            },
            provider_metadata: Some(provider_metadata),
        });
    };

    Ok(StreamResult {
        stream: Box::pin(stream),
        request_body: Some(body),
        response_headers: Some(response_headers),
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Convert an OpenAI stream-error JSON object into an `AiMuxError`.
///
/// The `code` field (when present) is used as the HTTP status code; `null`
/// defaults to 500 (server error, retryable). This mirrors the TS behaviour
/// where early stream errors reject the `doStream` promise with a status code.
fn stream_error_to_ai_error(err_obj: &Value) -> AiMuxError {
    let message = err_obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown stream error")
        .to_string();

    let code = err_obj.get("code").and_then(|v| v.as_u64()).unwrap_or(500);
    let status = code as u16;

    match status {
        401 => AiMuxError::Auth(message),
        429 => AiMuxError::RateLimited {
            retry_after_ms: 1000,
        },
        404 => AiMuxError::ModelNotFound(message),
        _ => AiMuxError::Provider(format!("HTTP {}: {}", status, message)),
    }
}
