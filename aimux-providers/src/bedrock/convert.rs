//! Conversion between `LanguageModelPrompt` and Amazon Bedrock Converse format.
//!
//! Mirrors `convert-to-amazon-bedrock-chat-messages.ts`,
//! `amazon-bedrock-prepare-tools.ts`, `map-amazon-bedrock-finish-reason.ts`,
//! and `convert-amazon-bedrock-usage.ts` in the TS SDK. The Converse API uses a
//! unified message format across all model providers:
//!
//! - System messages are lifted out of `messages` into a top-level `system`
//!   array of `{ "text": "..." }` blocks.
//! - User and tool messages are merged into `role: "user"` messages.
//! - Assistant messages become `role: "assistant"`.
//! - Tool calls become `toolUse` blocks; tool results become `toolResult`
//!   blocks.
//! - Consecutive same-role messages are merged into a single message (matching
//!   the TS `groupIntoBlocks` behaviour).

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified};
use base64::Engine;
use serde_json::{Value, json};

// ── Prompt conversion ───────────────────────────────────────────────────────

/// Convert a provider-facing prompt into Bedrock's `{ system, messages }` shape.
///
/// Returns `(system: Vec<Value>, messages: Vec<Value>)`.
///
/// Mirrors the TS `convertToAmazonBedrockChatMessages`: consecutive user+tool
/// messages are grouped into a single `user` block, consecutive assistant
/// messages into a single `assistant` block. Assistant text is trimmed when it
/// is the last content part of the last message of the last block; empty
/// assistant text is dropped unless the message also carries reasoning.
pub fn convert_prompt_to_bedrock(prompt: &LanguageModelPrompt) -> (Vec<Value>, Vec<Value>) {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Blk {
        System,
        User,
        Assistant,
    }

    // Group consecutive same-block messages (user+tool fold into User).
    let mut blocks: Vec<(Blk, Vec<usize>)> = Vec::new();
    for (i, msg) in prompt.iter().enumerate() {
        let b = match msg.role {
            Role::System => Blk::System,
            Role::User | Role::Tool => Blk::User,
            Role::Assistant => Blk::Assistant,
        };
        if blocks.last().map(|(lb, _)| *lb) == Some(b) {
            blocks.last_mut().unwrap().1.push(i);
        } else {
            blocks.push((b, vec![i]));
        }
    }

    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut document_counter: u32 = 0;
    let num_blocks = blocks.len();

    for (bi, (blk, idxs)) in blocks.iter().enumerate() {
        let is_last_block = bi == num_blocks - 1;
        match blk {
            Blk::System => {
                for &i in idxs {
                    let msg = &prompt[i];
                    for p in &msg.content {
                        if let ContentPart::Text { text, .. } = p {
                            system.push(json!({ "text": text }));
                        }
                        if let Some(cp) = part_cache_point(p) {
                            system.push(cp);
                        }
                    }
                }
            }
            Blk::User => {
                let mut content: Vec<Value> = Vec::new();
                for &i in idxs {
                    let msg = &prompt[i];
                    for p in &msg.content {
                        push_user_part(p, &mut content, &mut document_counter);
                        if let Some(cp) = part_cache_point(p) {
                            content.push(cp);
                        }
                    }
                }
                messages.push(json!({ "role": "user", "content": content }));
            }
            Blk::Assistant => {
                let mut content: Vec<Value> = Vec::new();
                let num_msgs = idxs.len();
                for (mj, &i) in idxs.iter().enumerate() {
                    let is_last_message = mj == num_msgs - 1;
                    let msg = &prompt[i];
                    let has_reasoning = msg
                        .content
                        .iter()
                        .any(|p| matches!(p, ContentPart::Reasoning { .. }));
                    let num_parts = msg.content.len();
                    for (kj, p) in msg.content.iter().enumerate() {
                        let is_last_content_part = kj == num_parts - 1;
                        match p {
                            ContentPart::Text { text, .. } => {
                                // Skip empty text unless the message has reasoning.
                                if text.trim().is_empty() && !has_reasoning {
                                    // skipped
                                } else {
                                    let t =
                                        if is_last_block && is_last_message && is_last_content_part
                                        {
                                            text.trim().to_string()
                                        } else {
                                            text.clone()
                                        };
                                    content.push(json!({ "text": t }));
                                }
                            }
                            ContentPart::Reasoning {
                                text,
                                signature: Some(sig),
                                ..
                            } => {
                                // Only signed reasoning is replayed; unsigned
                                // reasoning is intentionally omitted.
                                content.push(json!({
                                    "reasoningContent": {
                                        "reasoningText": {
                                            "text": text,
                                            "signature": sig
                                        }
                                    }
                                }));
                            }
                            ContentPart::Reasoning { .. } => {}
                            ContentPart::ToolCall {
                                tool_call_id,
                                tool_name,
                                input,
                                ..
                            } => {
                                let input_val = if input.is_object() {
                                    input.clone()
                                } else {
                                    json!({ "rawInvalidInput": input })
                                };
                                content.push(json!({
                                    "toolUse": {
                                        "toolUseId": tool_call_id,
                                        "name": sanitize_tool_name(tool_name),
                                        "input": input_val,
                                    }
                                }));
                            }
                            _ => {}
                        }
                        if let Some(cp) = part_cache_point(p) {
                            content.push(cp);
                        }
                    }
                }
                messages.push(json!({ "role": "assistant", "content": content }));
            }
        }
    }

    (system, messages)
}

/// Push a single user/tool content part into a Bedrock content array.
fn push_user_part(part: &ContentPart, content: &mut Vec<Value>, doc_counter: &mut u32) {
    match part {
        ContentPart::Text { text, .. } => {
            content.push(json!({ "text": text }));
        }
        ContentPart::Image {
            image, media_type, ..
        } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(image);
            push_file_block(&b64, media_type, None, &None, content, doc_counter);
        }
        ContentPart::File {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            push_file_block(
                &b64,
                media_type,
                filename.as_deref(),
                provider_options,
                content,
                doc_counter,
            );
        }
        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            provider_options,
        } => {
            // `data` is already a base64 string — use verbatim as `bytes`.
            push_file_block(
                data,
                media_type,
                filename.as_deref(),
                provider_options,
                content,
                doc_counter,
            );
        }
        ContentPart::ToolResult {
            tool_call_id,
            output,
            ..
        } => {
            let result_content = resolve_tool_result_output(output, doc_counter);
            content.push(json!({
                "toolResult": {
                    "toolUseId": tool_call_id,
                    "content": result_content,
                }
            }));
        }
        // Variants not yet modelled for Bedrock; no test exercises these paths.
        _ => {}
    }
}

/// Build a Bedrock `image` or `document` block from an already-base64 `bytes`
/// string. Images become `{ image: { format, source: { bytes } } }`; everything
/// else becomes a RAG `{ document: { format, name, source: { bytes } [, citations] } }`
/// block, mirroring the TS file-part handling (name = stripped filename or a
/// monotonically-incrementing `document-N`).
fn push_file_block(
    b64: &str,
    media_type: &str,
    filename: Option<&str>,
    provider_options: &Option<Value>,
    content: &mut Vec<Value>,
    doc_counter: &mut u32,
) {
    let top_level = media_type.split('/').next().unwrap_or("");
    if top_level == "image" {
        let format = mime_to_image_format(media_type);
        content.push(json!({
            "image": { "format": format, "source": { "bytes": b64 } }
        }));
    } else {
        let format = mime_to_document_format(media_type);
        let name = match filename {
            Some(f) => strip_file_extension(f),
            None => {
                *doc_counter += 1;
                format!("document-{}", *doc_counter)
            }
        };
        let mut doc = serde_json::Map::new();
        doc.insert("format".to_string(), json!(format));
        doc.insert("name".to_string(), json!(name));
        doc.insert("source".to_string(), json!({ "bytes": b64 }));
        if citations_enabled(provider_options) {
            doc.insert("citations".to_string(), json!({ "enabled": true }));
        }
        content.push(json!({ "document": Value::Object(doc) }));
    }
}

/// Extract a `{ cachePoint: {...} }` block from a part's `providerOptions`
/// (`bedrock.cachePoint` or `amazonBedrock.cachePoint`), if present.
fn part_cache_point(part: &ContentPart) -> Option<Value> {
    let po = match part {
        ContentPart::Text {
            provider_options, ..
        }
        | ContentPart::Image {
            provider_options, ..
        }
        | ContentPart::File {
            provider_options, ..
        }
        | ContentPart::FileBase64 {
            provider_options, ..
        } => provider_options.as_ref()?,
        _ => return None,
    };
    for key in &["bedrock", "amazonBedrock"] {
        if let Some(cp) = po.get(key).and_then(|v| v.get("cachePoint")) {
            return Some(json!({ "cachePoint": cp.clone() }));
        }
    }
    None
}

/// Whether `citations.enabled` is set on a part's `bedrock`/`amazonBedrock`
/// provider options.
fn citations_enabled(provider_options: &Option<Value>) -> bool {
    let Some(po) = provider_options.as_ref() else {
        return false;
    };
    for key in &["bedrock", "amazonBedrock"] {
        if let Some(enabled) = po
            .get(key)
            .and_then(|b| b.get("citations"))
            .and_then(|c| c.get("enabled"))
            .and_then(|e| e.as_bool())
        {
            return enabled;
        }
    }
    false
}

/// Resolve a tool result `output` value into Bedrock's `toolResult.content`
/// array, mirroring the TS SDK.
fn resolve_tool_result_output(output: &Value, doc_counter: &mut u32) -> Vec<Value> {
    let (t, v) = match (
        output.get("type").and_then(|x| x.as_str()),
        output.get("value"),
    ) {
        (Some(t), Some(v)) => (t, v),
        _ => return vec![json!({ "text": output.to_string() })],
    };
    match t {
        "json" | "error-json" => vec![json!({ "text": v.to_string() })],
        "text" | "error" | "error-text" => vec![json!({ "text": v })],
        "execution-denied" => {
            let reason = v.as_str().unwrap_or("Tool call execution denied.");
            vec![json!({ "text": reason })]
        }
        "content" => {
            if let Some(arr) = v.as_array() {
                let mut out = Vec::new();
                for part in arr {
                    convert_tool_result_content_part(part, &mut out, doc_counter);
                }
                out
            } else {
                vec![json!({ "text": output.to_string() })]
            }
        }
        _ => vec![json!({ "text": output.to_string() })],
    }
}

/// Convert a single content part inside a tool-result `content` array.
fn convert_tool_result_content_part(part: &Value, content: &mut Vec<Value>, doc_counter: &mut u32) {
    let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match part_type {
        "text" => {
            let text = part.get("text").and_then(|t| t.as_str()).unwrap_or("");
            content.push(json!({ "text": text }));
        }
        "file" => {
            let media_type = part.get("mediaType").and_then(|m| m.as_str()).unwrap_or("");
            // Tool-result file parts carry an already-base64 `data` string.
            let b64 = part
                .get("data")
                .and_then(|d| d.get("data"))
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let filename = part.get("filename").and_then(|f| f.as_str());
            push_file_block(b64, media_type, filename, &None, content, doc_counter);
        }
        _ => {
            content.push(json!({ "text": part.to_string() }));
        }
    }
}

fn sanitize_tool_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn strip_file_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

fn mime_to_image_format(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpeg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn mime_to_document_format(media_type: &str) -> &'static str {
    match media_type {
        "application/pdf" => "pdf",
        "text/csv" => "csv",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "text/html" => "html",
        "text/plain" => "txt",
        "text/markdown" => "md",
        _ => "txt",
    }
}

// ── Tools ───────────────────────────────────────────────────────────────────

/// Whether a Bedrock model id supports the `strict` tool schema field.
///
/// Mirrors the TS `supportsStrictTools`: the newest Claude models reject
/// `strict` (and `output_config.format`), so the field is omitted for them.
pub fn supports_strict_tools(model_id: &str) -> bool {
    const REJECTING: &[&str] = &[
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-fable-5",
        "claude-sonnet-5",
    ];
    !REJECTING.iter().any(|m| model_id.contains(m))
}

/// Prepare `FunctionTool`s into the Bedrock `toolConfig` JSON shape.
///
/// Mirrors the function-tool subset of the TS `prepareTools`:
/// - no tools (or `toolChoice: none`) → empty `toolConfig` `{}`
/// - `toolChoice: tool` filters tools to the named one
/// - `toolChoice: auto/required/tool` → `{ auto: {} }` / `{ any: {} }` /
///   `{ tool: { name } }`
/// - `description` is omitted when empty/whitespace
/// - `strict` is passed through only for models that `supports_strict_tools`
///
/// Provider-defined tools (web_search, anthropic provider tools) and the
/// `additionalTools`/`betas`/`toolWarnings` they produce are not modelled in
/// the Rust `FunctionTool` and are intentionally not handled here.
pub fn prepare_tools(
    tools: &Option<Vec<FunctionTool>>,
    tool_choice: &ToolChoice,
    model_id: &str,
) -> Value {
    let non_empty = tools.as_ref().filter(|t| !t.is_empty());
    let Some(tools) = non_empty else {
        return json!({});
    };

    // `toolChoice: none` clears tools entirely (matches TS).
    if matches!(tool_choice, ToolChoice::None) {
        return json!({});
    }

    // `toolChoice: tool` filters function tools to the named one.
    let filtered: Vec<&FunctionTool> = match tool_choice {
        ToolChoice::Tool { tool_name } => tools.iter().filter(|t| &t.name == tool_name).collect(),
        _ => tools.iter().collect(),
    };

    let supports_strict = supports_strict_tools(model_id);
    let tool_specs: Vec<Value> = filtered
        .iter()
        .map(|t| {
            let mut spec = serde_json::Map::new();
            spec.insert("name".to_string(), json!(t.name));
            if let Some(ref desc) = t.description
                && !desc.trim().is_empty()
            {
                spec.insert("description".to_string(), json!(desc));
            }
            if let Some(strict) = t.strict
                && supports_strict
            {
                spec.insert("strict".to_string(), json!(strict));
            }
            spec.insert("inputSchema".to_string(), json!({ "json": t.input_schema }));
            json!({ "toolSpec": Value::Object(spec) })
        })
        .collect();

    if tool_specs.is_empty() {
        return json!({});
    }

    let tool_choice_val = match tool_choice {
        ToolChoice::Auto => json!({ "auto": {} }),
        ToolChoice::Required => json!({ "any": {} }),
        ToolChoice::Tool { tool_name } => json!({ "tool": { "name": tool_name } }),
        ToolChoice::None => unreachable!(),
    };

    json!({ "tools": tool_specs, "toolChoice": tool_choice_val })
}

// ── Request body ────────────────────────────────────────────────────────────

/// Build the Bedrock Converse request body.
///
/// When `providerOptions.bedrock.reasoningConfig = { type: 'enabled',
/// budgetTokens: N }` is present, it is translated into
/// `additionalModelRequestFields.thinking = { type: 'enabled', budget_tokens: N }`
/// and `inferenceConfig.maxTokens` is bumped by `budgetTokens`. The
/// `reasoningConfig` key never appears in the request body. User-supplied
/// `additionalModelRequestFields` are merged with the derived `thinking` field.
pub fn build_request_body(model_id: &str, options: &CallOptions) -> Value {
    let (system, messages) = convert_prompt_to_bedrock(&options.prompt);

    // Extract Bedrock-specific provider options.
    let bedrock_opts = options
        .provider_options
        .as_ref()
        .and_then(|po| po.get("bedrock"));
    let reasoning_config = bedrock_opts.and_then(|bo| bo.get("reasoningConfig"));
    let user_amrf = bedrock_opts.and_then(|bo| bo.get("additionalModelRequestFields"));

    // Derive the thinking config and budget_tokens from reasoningConfig.
    let mut thinking: Option<Value> = None;
    let mut budget_tokens: Option<u64> = None;
    if let Some(rc) = reasoning_config
        && rc.get("type").and_then(|v| v.as_str()) == Some("enabled")
        && let Some(bt) = rc.get("budgetTokens").and_then(|v| v.as_u64())
    {
        thinking = Some(json!({ "type": "enabled", "budget_tokens": bt }));
        budget_tokens = Some(bt);
    }

    let mut inference_config = serde_json::Map::new();
    if let Some(max) = options.max_output_tokens {
        // When thinking is enabled, maxTokens is bumped by budgetTokens so the
        // model has room for both reasoning and the visible response.
        let max_tokens = budget_tokens.map(|bt| max + bt as u32).unwrap_or(max);
        inference_config.insert("maxTokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = options.temperature {
        inference_config.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        inference_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(top_k) = options.top_k {
        inference_config.insert("topK".to_string(), json!(top_k));
    }
    if let Some(ref stop) = options.stop_sequences {
        inference_config.insert("stopSequences".to_string(), json!(stop));
    }

    // Build additionalModelRequestFields: merge user-supplied fields with the
    // derived thinking config (thinking takes precedence on conflict).
    let mut amrf = serde_json::Map::new();
    if let Some(user_amrf) = user_amrf
        && let Some(obj) = user_amrf.as_object()
    {
        for (k, v) in obj {
            amrf.insert(k.clone(), v.clone());
        }
    }
    if let Some(thinking) = thinking {
        amrf.insert("thinking".to_string(), thinking);
    }

    let mut body = serde_json::Map::new();
    body.insert("messages".to_string(), Value::Array(messages));

    if !system.is_empty() {
        body.insert("system".to_string(), Value::Array(system));
    }
    if !inference_config.is_empty() {
        body.insert(
            "inferenceConfig".to_string(),
            Value::Object(inference_config),
        );
    }
    if !amrf.is_empty() {
        body.insert(
            "additionalModelRequestFields".to_string(),
            Value::Object(amrf),
        );
    }

    // Tools
    let function_tools: Option<Vec<FunctionTool>> = options.tools.as_ref().map(|tools| {
        tools
            .iter()
            .filter_map(|t| match t {
                Tool::Function(ft) => Some(ft.clone()),
                Tool::Provider(_) => None,
            })
            .collect()
    });
    let tool_config = prepare_tools(&function_tools, &options.tool_choice, model_id);
    if tool_config
        .as_object()
        .map(|o| !o.is_empty())
        .unwrap_or(false)
    {
        body.insert("toolConfig".to_string(), tool_config);
    }

    Value::Object(body)
}

/// Map a Bedrock `stopReason` to the unified `FinishReason`.
pub fn map_finish_reason(reason: &str) -> FinishReason {
    let unified = match reason {
        "stop_sequence" | "end_turn" | "stop" => FinishReasonUnified::Stop,
        "max_tokens" | "length" => FinishReasonUnified::Length,
        "content_filtered" | "content-filter" | "guardrail_intervened" => {
            FinishReasonUnified::ContentFilter
        }
        "tool_use" | "tool-calls" => FinishReasonUnified::ToolCalls,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(reason.to_string()),
    }
}

/// Convert Bedrock usage to the core `Usage` type.
///
/// Mirrors the TS `convertAmazonBedrockUsage(usage: AmazonBedrockUsage |
/// undefined | null)`: a `None`/null/undefined usage yields an all-`None`
/// `Usage` (the TS `undefined` fields). The TS `raw` echo is not modelled on
/// the Rust `Usage` type (see `convert-usage` tests for the skipped `raw`
/// cases); `outputTokens.text` mirrors the TS `outputTokens.text` field.
pub fn convert_usage(usage: Option<&BedrockUsage>) -> aimux_core::types::Usage {
    use aimux_core::types::{TokenUsage, Usage};

    let Some(usage) = usage else {
        return Usage::default();
    };

    let input = usage.input_tokens.unwrap_or(0);
    let output = usage.output_tokens.unwrap_or(0);
    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_write = usage.cache_write_input_tokens.unwrap_or(0);

    Usage {
        input_tokens: TokenUsage {
            total: Some(input + cache_read + cache_write),
            no_cache: Some(input),
            cache_read: Some(cache_read),
            cache_write: Some(cache_write),
            ..Default::default()
        },
        output_tokens: TokenUsage {
            total: Some(output),
            text: Some(output),
            ..Default::default()
        },
    }
}

// Re-export for convenience.
pub use super::types::BedrockUsage;
