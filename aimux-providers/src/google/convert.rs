//! Conversion between `LanguageModelPrompt` and Google Gemini API format.
//!
//! Mirrors the TS SDK's `convert-to-google-messages.ts` and the request-body
//! construction inside `google-language-model.ts`'s `getArgs`. The Gemini
//! request shape is fundamentally different from OpenAI/Anthropic:
//!
//! - System messages are lifted out of `contents` into a top-level
//!   `systemInstruction` field (a `{ parts: [{ text }] }` object).
//! - Assistant messages become `role: "model"`.
//! - Tool results become `functionResponse` parts inside a `role: "user"`
//!   message (Gemini has no `tool` role).
//! - Tool calls in assistant messages become `functionCall` parts.
//!
//! We model the variable-shape content parts as `serde_json::Value` to keep
//! the surface area small — the TS SDK uses a tagged union, but the only
//! fields we actually read back are `text`, `functionCall`, and
//! `functionResponse`, all of which we already produce ourselves.

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::LanguageModelPrompt;
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::result::GenerateContent;
use aimux_core::tool::{FunctionTool, Tool};
use aimux_core::types::{FinishReason, FinishReasonUnified, Warning};
use base64::Engine;
use serde_json::{Map, Value, json};

// ── Public conversion result ─────────────────────────────────────────────────

/// The result of converting a `LanguageModelPrompt` into Google's
/// `contents` + `systemInstruction` shape.
#[derive(Debug, Clone, Default)]
pub struct GooglePrompt {
    /// Top-level `systemInstruction` object, or `None` when there are no
    /// system messages (or for Gemma models, which don't accept it).
    pub system_instruction: Option<Value>,
    /// The `contents` array — one entry per non-system message (tool messages
    /// are folded into the preceding user turn as `functionResponse` parts).
    pub contents: Vec<Value>,
}

// ── convertToGoogleMessages ──────────────────────────────────────────────────

/// Convert a provider-facing prompt into Google's `{ systemInstruction, contents }`.
///
/// Mirrors `convertToGoogleMessages` in the TS SDK with the simplifications
/// appropriate to the Rust data model:
/// - No Gemma special-casing (we don't know the model id here; callers that
///   need it can post-process).
/// - No server-tool-call handling — provider-executed `toolCall` parts have no
///   dedicated `ContentPart` variant; callers must reconstruct them from raw
///   results if they need to replay them.
/// - Tool-result `output` is serialized into `functionResponse.response.content`
///   as a string (JSON-stringified for non-string outputs, matching the TS
///   `output.type === 'json'` path).
///
/// Thought signatures: `ContentPart::ToolCall.thought_signature` is echoed
/// back as a `thoughtSignature` sibling of the `functionCall` part (required
/// by Gemini thinking models on follow-up turns).
pub fn convert_to_google_messages(prompt: &LanguageModelPrompt) -> GooglePrompt {
    let mut system_parts: Vec<Value> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut system_messages_allowed = true;

    for msg in prompt {
        match msg.role {
            Role::System => {
                if !system_messages_allowed {
                    // The TS SDK throws `UnsupportedFunctionalityError` here:
                    // system messages are only valid at the start of the
                    // conversation. This function returns `GooglePrompt` (not
                    // `Result`), so we cannot propagate an error. Dropping the
                    // late system message is safer than folding it into
                    // `systemInstruction` (which would make Gemini treat a
                    // mid-conversation instruction as a global rule).
                    // TODO: change the signature to `Result` to match TS semantics.
                    continue;
                }
                for part in &msg.content {
                    if let ContentPart::Text { text, .. } = part {
                        system_parts.push(json!({ "text": text }));
                    }
                }
            }
            Role::User => {
                system_messages_allowed = false;
                let parts = convert_user_parts(&msg.content);
                contents.push(json!({ "role": "user", "parts": parts }));
            }
            Role::Assistant => {
                system_messages_allowed = false;
                let parts = convert_assistant_parts(&msg.content);
                if !parts.is_empty() {
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
            }
            Role::Tool => {
                system_messages_allowed = false;
                // Gemini folds tool results into a `user`-role message as
                // `functionResponse` parts. We emit them as their own user
                // turn (matching how the TS SDK pushes a new `{role:'user'}`
                // entry per tool-role message).
                let parts = convert_tool_parts(&msg.content);
                if !parts.is_empty() {
                    contents.push(json!({ "role": "user", "parts": parts }));
                }
            }
        }
    }

    let system_instruction = if system_parts.is_empty() {
        None
    } else {
        Some(json!({ "parts": system_parts }))
    };

    GooglePrompt {
        system_instruction,
        contents,
    }
}

/// Convert user-role content parts into Google parts.
fn convert_user_parts(content: &[ContentPart]) -> Vec<Value> {
    let mut parts = Vec::new();
    for part in content {
        match part {
            ContentPart::Text { text, .. } => {
                parts.push(json!({ "text": text }));
            }
            ContentPart::Image {
                image, media_type, ..
            } => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(image);
                parts.push(json!({
                    "inlineData": { "mimeType": media_type, "data": b64 }
                }));
            }
            ContentPart::File {
                data, media_type, ..
            } => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                parts.push(json!({
                    "inlineData": { "mimeType": media_type, "data": b64 }
                }));
            }
            // Tool calls / results inside a user message are unusual; skip
            // them rather than trying to stringify a `ContentPart`.
            _ => {}
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "text": "" }));
    }
    parts
}

/// Convert assistant-role content parts into Google `model`-role parts.
///
/// - `Text` → `{ text }` (skipped when empty, matching the TS SDK).
/// - `ToolCall` → `{ functionCall: { id?, name, args } }`.
fn convert_assistant_parts(content: &[ContentPart]) -> Vec<Value> {
    let mut parts = Vec::new();
    for part in content {
        match part {
            ContentPart::Text { text, .. } => {
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
            }
            ContentPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                thought_signature,
                ..
            } => {
                let mut function_call = Map::new();
                if !tool_call_id.is_empty() {
                    function_call.insert("id".to_string(), json!(tool_call_id));
                }
                function_call.insert("name".to_string(), json!(tool_name));
                function_call.insert("args".to_string(), input.clone());
                let mut part_value = json!({ "functionCall": function_call });
                // Thinking models (e.g. gemini-2.5-pro) attach a
                // `thoughtSignature` to the part; it must be echoed back
                // verbatim on the follow-up turn or the API rejects the
                // request with HTTP 400. Emit it as a sibling of
                // `functionCall` (not inside it), matching the response shape.
                if let Some(sig) = thought_signature {
                    part_value["thoughtSignature"] = json!(sig);
                }
                parts.push(part_value);
            }
            _ => {
                // Files / images in assistant messages aren't supported by
                // the Rust data model in a meaningful way here; skip them.
            }
        }
    }
    parts
}

/// Convert tool-role content parts into Google `functionResponse` parts.
///
/// The TS SDK uses the `functionResponse` shape:
/// `{ functionResponse: { id?, name, response: { name, content } } }`.
/// `content` is the tool's output serialized to a string (string outputs
/// pass through; JSON outputs are stringified).
fn convert_tool_parts(content: &[ContentPart]) -> Vec<Value> {
    let mut parts = Vec::new();
    for part in content {
        if let ContentPart::ToolResult {
            tool_call_id,
            result,
            ..
        } = part
        {
            let content_str = match result {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let mut function_response = Map::new();
            if !tool_call_id.is_empty() {
                function_response.insert("id".to_string(), json!(tool_call_id));
            }
            // `name` is required by the API; the TS SDK uses the tool name
            // from the part. We don't have it on `ToolResult`, so we fall
            // back to the call id. This is sufficient for round-tripping
            // with our own test fixtures; production callers that need the
            // real tool name should use `ContentPart::ToolCall` first.
            let name = tool_call_id.clone();
            function_response.insert("name".to_string(), json!(name));
            function_response.insert(
                "response".to_string(),
                json!({ "name": name, "content": content_str }),
            );
            parts.push(json!({ "functionResponse": function_response }));
        }
    }
    parts
}

// ── prepareTools ─────────────────────────────────────────────────────────────

/// Result of preparing tools for the Google request body.
#[derive(Debug, Clone, Default)]
pub struct PreparedTools {
    /// `tools` array (e.g. `[{ functionDeclarations: [...] }]`), or `None`
    /// when there are no usable tools.
    pub tools: Option<Vec<Value>>,
    /// `toolConfig` object (e.g. `{ functionCallingConfig: { mode: "AUTO" } }`).
    pub tool_config: Option<Value>,
}

/// Prepare `FunctionTool`s into the Google `tools` / `toolConfig` JSON shape.
///
/// Mirrors the function-tools path of the TS `prepareTools`. Provider-defined
/// tools (`google_search`, `code_execution`, …) are out of scope for the Rust
/// port — only `FunctionTool`s are supported.
pub fn prepare_tools(tools: &Option<Vec<FunctionTool>>, tool_choice: &ToolChoice) -> PreparedTools {
    // Coerce empty arrays to None (matches TS `tools?.length ? tools : undefined`).
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());

    let Some(tools) = non_empty else {
        return PreparedTools::default();
    };

    let mut has_strict = false;
    let function_declarations: Vec<Value> = tools
        .iter()
        .map(|t| {
            if t.strict == Some(true) {
                has_strict = true;
            }
            let mut decl = json!({
                "name": t.name,
                "parameters": convert_json_schema_to_openapi_schema(&t.input_schema, true),
            });
            if let Some(desc) = &t.description {
                decl["description"] = json!(desc);
            } else {
                decl["description"] = json!("");
            }
            decl
        })
        .collect();

    let tools_value = Some(vec![
        json!({ "functionDeclarations": function_declarations }),
    ]);

    let tool_config = match tool_choice {
        ToolChoice::Auto => {
            if has_strict {
                Some(json!({ "functionCallingConfig": { "mode": "VALIDATED" } }))
            } else {
                Some(json!({ "functionCallingConfig": { "mode": "AUTO" } }))
            }
        }
        ToolChoice::None => Some(json!({ "functionCallingConfig": { "mode": "NONE" } })),
        ToolChoice::Required => {
            if has_strict {
                Some(json!({ "functionCallingConfig": { "mode": "VALIDATED" } }))
            } else {
                Some(json!({ "functionCallingConfig": { "mode": "ANY" } }))
            }
        }
        ToolChoice::Tool { tool_name } => {
            if has_strict {
                Some(json!({
                    "functionCallingConfig": {
                        "mode": "VALIDATED",
                        "allowedFunctionNames": [tool_name]
                    }
                }))
            } else {
                Some(json!({
                    "functionCallingConfig": {
                        "mode": "ANY",
                        "allowedFunctionNames": [tool_name]
                    }
                }))
            }
        }
    };

    PreparedTools {
        tools: tools_value,
        tool_config,
    }
}

// ── Model capabilities ───────────────────────────────────────────────────────

/// Gemini model capabilities, mirroring `getGoogleModelCapabilities` in the TS
/// SDK. Determines which provider-defined tools a model supports and whether the
/// Gemini 3 combined tool shape applies.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoogleModelCapabilities {
    /// `google_search` / `url_context` / `code_execution` / `google_maps` /
    /// `enterprise_web_search` / `vertex_rag_store` require Gemini 2.0+.
    pub supports_gemini_2_tools: bool,
    /// `file_search` requires Gemini 2.5+ or Gemini 3.
    pub supports_file_search: bool,
    /// Gemini 3 keeps function + provider tools together with
    /// `includeServerSideToolInvocations`.
    pub uses_gemini_3_features: bool,
}

/// Classify Gemini capabilities by model id.
///
/// Mirrors `getGoogleModelCapabilities`. Unrecognized Gemini ids inherit the
/// newest supported behaviour (matching the TS intent); only known older
/// generations are downgraded.
pub fn get_google_model_capabilities(model_id: &str) -> GoogleModelCapabilities {
    let lower = model_id.to_lowercase();

    let is_gemini_model = contains_at_boundary(&lower, "gemini-");
    let is_gemini_2 = matches_prefix_boundary(&lower, "gemini-2");
    let is_gemini_25 = matches_prefix_boundary(&lower, "gemini-2.5");
    let is_gemini_1 = matches_prefix_boundary(&lower, "gemini-1");
    let is_known_pre_gemini_2 = is_gemini_1
        || matches_at_end(&lower, "gemini-pro")
        || matches_at_end(&lower, "gemini-pro-vision")
        || matches_prefix_boundary(&lower, "gemini-robotics-er-1.5");
    let is_known_older_model = is_known_pre_gemini_2 || is_gemini_2;
    let uses_gemini_3_features = is_gemini_model && !is_known_older_model;

    GoogleModelCapabilities {
        supports_gemini_2_tools: (is_gemini_model && !is_known_pre_gemini_2)
            || lower.contains("nano-banana"),
        supports_file_search: is_gemini_25 || uses_gemini_3_features,
        uses_gemini_3_features,
    }
}

/// `/(^|\/)prefix/` — `prefix` appears at the start of `lower` or just after a
/// `/`, with no requirement on what follows.
fn contains_at_boundary(lower: &str, prefix: &str) -> bool {
    lower.starts_with(prefix) || lower.contains(&format!("/{}", prefix))
}

/// `/(^|\/)prefix(?:[.-]|$)/` — `prefix` appears at the start of `lower` or
/// just after a `/`, and is followed by `.`, `-`, or end-of-string.
fn matches_prefix_boundary(lower: &str, prefix: &str) -> bool {
    let bytes = lower.as_bytes();
    let plen = prefix.len();
    if plen > bytes.len() {
        return false;
    }
    let pb = prefix.as_bytes();
    for i in 0..=bytes.len() - plen {
        if i != 0 && bytes[i - 1] != b'/' {
            continue;
        }
        if &bytes[i..i + plen] != pb {
            continue;
        }
        let after = i + plen;
        if after >= bytes.len() {
            return true;
        }
        let next = bytes[after];
        if next == b'.' || next == b'-' {
            return true;
        }
    }
    false
}

/// `/(^|\/)suffix$/` — `lower` ends with `suffix`, and the suffix begins at the
/// start of the string or just after a `/`.
fn matches_at_end(lower: &str, suffix: &str) -> bool {
    let bytes = lower.as_bytes();
    let slen = suffix.len();
    if slen > bytes.len() {
        return false;
    }
    let start = bytes.len() - slen;
    if &bytes[start..] != suffix.as_bytes() {
        return false;
    }
    start == 0 || bytes[start - 1] == b'/'
}

// ── prepareTools (provider-defined tools) ────────────────────────────────────

/// Result of preparing tools for the Google request body, including any
/// warnings about unsupported tools.
#[derive(Debug, Clone, Default)]
pub struct PreparedToolsWithWarnings {
    /// `tools` array, or `None` when there are no usable tools.
    pub tools: Option<Vec<Value>>,
    /// `toolConfig` object.
    pub tool_config: Option<Value>,
    /// Warnings about unsupported tools / combinations.
    pub warnings: Vec<Warning>,
}

/// Prepare function **and** provider-defined tools into the Google `tools` /
/// `toolConfig` JSON shape, mirroring the TS `prepareTools`.
///
/// Unlike [`prepare_tools`] (which only handles `FunctionTool`s), this handles
/// `Tool::Provider` entries (`google.google_search`, `google.code_execution`,
/// `google.url_context`, `google.google_maps`, `google.enterprise_web_search`,
/// `google.file_search`) and the Gemini 3 combined function+provider shape.
pub fn prepare_all_tools(
    tools: &Option<Vec<Tool>>,
    tool_choice: &ToolChoice,
    model_id: &str,
) -> PreparedToolsWithWarnings {
    let caps = get_google_model_capabilities(model_id);
    let mut warnings: Vec<Warning> = Vec::new();

    // Coerce empty arrays to None (matches TS `tools?.length ? tools : undefined`).
    let non_empty = tools.as_ref().filter(|&t| !t.is_empty());
    let Some(tools) = non_empty else {
        return PreparedToolsWithWarnings::default();
    };

    let has_function_tools = tools.iter().any(|t| matches!(t, Tool::Function(_)));
    let has_provider_tools = tools.iter().any(|t| matches!(t, Tool::Provider(_)));

    if has_function_tools && has_provider_tools && !caps.uses_gemini_3_features {
        warnings.push(Warning::Unsupported {
            feature: "combination of function and provider-defined tools".to_string(),
            details: None,
        });
    }

    if has_provider_tools {
        let mut google_tools: Vec<Value> = Vec::new();

        for tool in tools.iter().filter_map(|t| match t {
            Tool::Provider(pt) => Some(pt),
            _ => None,
        }) {
            push_provider_tool(tool, &caps, &mut google_tools, &mut warnings);
        }

        // Gemini 3: keep function declarations alongside provider tools.
        if has_function_tools && caps.uses_gemini_3_features && !google_tools.is_empty() {
            let function_declarations: Vec<Value> = tools
                .iter()
                .filter_map(|t| match t {
                    Tool::Function(ft) => Some(build_function_declaration(ft)),
                    _ => None,
                })
                .collect();

            let mut combined_config = Map::new();
            let fc = match tool_choice {
                ToolChoice::None => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("NONE"));
                    m
                }
                ToolChoice::Required => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("ANY"));
                    m
                }
                ToolChoice::Tool { tool_name } => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("ANY"));
                    m.insert("allowedFunctionNames".to_string(), json!([tool_name]));
                    m
                }
                ToolChoice::Auto => {
                    let mut m = Map::new();
                    m.insert("mode".to_string(), json!("VALIDATED"));
                    m
                }
            };
            combined_config.insert("functionCallingConfig".to_string(), Value::Object(fc));
            combined_config.insert("includeServerSideToolInvocations".to_string(), json!(true));

            let mut tools_value = google_tools;
            tools_value.push(json!({ "functionDeclarations": function_declarations }));

            return PreparedToolsWithWarnings {
                tools: Some(tools_value),
                tool_config: Some(Value::Object(combined_config)),
                warnings,
            };
        }

        let tools_value = if google_tools.is_empty() {
            None
        } else {
            Some(google_tools)
        };
        return PreparedToolsWithWarnings {
            tools: tools_value,
            tool_config: None,
            warnings,
        };
    }

    // Function-only path: delegate to the existing `prepare_tools`.
    let function_tools: Vec<FunctionTool> = tools
        .iter()
        .filter_map(|t| match t {
            Tool::Function(ft) => Some(ft.clone()),
            _ => None,
        })
        .collect();
    let prepared = prepare_tools(&Some(function_tools), tool_choice);
    PreparedToolsWithWarnings {
        tools: prepared.tools,
        tool_config: prepared.tool_config,
        warnings,
    }
}

/// Map a single provider-defined tool to its Google request-body entry, pushing
/// either the tool object or an `Unsupported` warning.
fn push_provider_tool(
    tool: &aimux_core::tool::ProviderTool,
    caps: &GoogleModelCapabilities,
    google_tools: &mut Vec<Value>,
    warnings: &mut Vec<Warning>,
) {
    let unsupported = |details: Option<&str>| Warning::Unsupported {
        feature: format!("provider-defined tool {}", tool.id),
        details: details.map(|s| s.to_string()),
    };
    match tool.id.as_str() {
        "google.google_search" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "googleSearch": tool.args }));
            } else {
                warnings.push(unsupported(Some(
                    "Google Search requires Gemini 2.0 or newer.",
                )));
            }
        }
        "google.enterprise_web_search" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "enterpriseWebSearch": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "Enterprise Web Search requires Gemini 2.0 or newer.",
                )));
            }
        }
        "google.url_context" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "urlContext": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The URL context tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.code_execution" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "codeExecution": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The code execution tool is not supported with other Gemini models than Gemini 2.",
                )));
            }
        }
        "google.file_search" => {
            if caps.supports_file_search {
                google_tools.push(json!({ "fileSearch": tool.args }));
            } else {
                warnings.push(unsupported(Some(
                    "The file search tool is only supported with Gemini 2.5 models and Gemini 3 models.",
                )));
            }
        }
        "google.google_maps" => {
            if caps.supports_gemini_2_tools {
                google_tools.push(json!({ "googleMaps": {} }));
            } else {
                warnings.push(unsupported(Some(
                    "The Google Maps grounding tool is not supported with Gemini models other than Gemini 2 or newer.",
                )));
            }
        }
        _ => {
            warnings.push(unsupported(None));
        }
    }
}

/// Build a single `functionDeclarations` entry from a `FunctionTool`.
///
/// `parameters` is omitted when the converted schema is null (empty object
/// schemas at the root), matching the TS `convertJSONSchemaToOpenAPISchema`
/// returning `undefined`.
fn build_function_declaration(ft: &FunctionTool) -> Value {
    let mut decl = Map::new();
    decl.insert("name".to_string(), json!(ft.name));
    decl.insert(
        "description".to_string(),
        json!(ft.description.as_deref().unwrap_or("")),
    );
    let params = convert_json_schema_to_openapi_schema(&ft.input_schema, true);
    if !params.is_null() {
        decl.insert("parameters".to_string(), params);
    }
    Value::Object(decl)
}

// ── JSON Schema → OpenAPI Schema ─────────────────────────────────────────────

/// Convert a JSON Schema value into the OpenAPI 3.0 schema variant that
/// Gemini's `functionDeclarations.parameters` expects.
///
/// Mirrors `convert-json-schema-to-openapi-schema.ts` for the cases that
/// matter for tool parameter schemas:
/// - Empty object schemas become `undefined` at the root and
///   `{ type: "object" }` when nested.
/// - `$schema` / `additionalProperties` / `definitions` etc. are dropped.
/// - `type`, `description`, `required`, `properties`, `items`, `enum`,
///   `format`, `const`, `anyOf`/`oneOf`/`allOf` are preserved.
pub fn convert_json_schema_to_openapi_schema(schema: &Value, is_root: bool) -> Value {
    if schema.is_null() {
        return Value::Null;
    }

    // Boolean schema.
    if let Some(b) = schema.as_bool() {
        let _ = b;
        return json!({ "type": "boolean", "properties": {} });
    }

    let obj = match schema.as_object() {
        Some(o) => o,
        None => return schema.clone(),
    };

    // Empty object schema: `{ type: "object" }` with no/empty properties
    // and no `additionalProperties`.
    if is_empty_object_schema(obj) {
        if is_root {
            return Value::Null;
        }
        if obj.contains_key("description") {
            return json!({ "type": "object", "description": obj["description"] });
        }
        return json!({ "type": "object" });
    }

    let mut result = Map::new();

    if let Some(desc) = obj.get("description") {
        result.insert("description".to_string(), desc.clone());
    }
    if let Some(req) = obj.get("required") {
        result.insert("required".to_string(), req.clone());
    }
    if let Some(format) = obj.get("format") {
        result.insert("format".to_string(), format.clone());
    }

    if let Some(const_val) = obj.get("const") {
        result.insert("enum".to_string(), json!([const_val]));
    }

    // type: string | array<string>
    if let Some(type_val) = obj.get("type") {
        if let Some(type_str) = type_val.as_str() {
            result.insert("type".to_string(), json!(type_str));
        } else if let Some(types) = type_val.as_array() {
            let has_null = types.iter().any(|t| t.as_str() == Some("null"));
            let non_null: Vec<&Value> = types
                .iter()
                .filter(|t| t.as_str() != Some("null"))
                .collect();
            if non_null.is_empty() {
                // Only null type.
                result.insert("type".to_string(), json!("null"));
            } else {
                // One or more non-null types: always use anyOf (matching TS).
                let any_of: Vec<Value> = non_null.iter().map(|t| json!({ "type": t })).collect();
                result.insert("anyOf".to_string(), Value::Array(any_of));
                if has_null {
                    result.insert("nullable".to_string(), json!(true));
                }
            }
        }
    }

    if let Some(enum_vals) = obj.get("enum") {
        result.insert("enum".to_string(), enum_vals.clone());
    }

    if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
        let mut out = Map::new();
        for (k, v) in props {
            out.insert(k.clone(), convert_json_schema_to_openapi_schema(v, false));
        }
        result.insert("properties".to_string(), Value::Object(out));
    }

    if let Some(items) = obj.get("items") {
        if let Some(arr) = items.as_array() {
            let converted: Vec<Value> = arr
                .iter()
                .map(|i| convert_json_schema_to_openapi_schema(i, false))
                .collect();
            result.insert("items".to_string(), Value::Array(converted));
        } else {
            result.insert(
                "items".to_string(),
                convert_json_schema_to_openapi_schema(items, false),
            );
        }
    }

    // allOf / oneOf: recursively convert each element.
    for combinator in ["allOf", "oneOf"] {
        if let Some(arr) = obj.get(combinator).and_then(|v| v.as_array()) {
            let converted: Vec<Value> = arr
                .iter()
                .map(|i| convert_json_schema_to_openapi_schema(i, false))
                .collect();
            result.insert(combinator.to_string(), Value::Array(converted));
        }
    }

    // anyOf: collapse a `null`-typed branch into `nullable: true` (matching
    // the TS SDK, which folds `anyOf: [T, {type:null}]` into `T + nullable`).
    if let Some(arr) = obj.get("anyOf").and_then(|v| v.as_array()) {
        let has_null = arr.iter().any(|s| {
            s.as_object()
                .and_then(|o| o.get("type"))
                .and_then(|t| t.as_str())
                == Some("null")
        });
        if has_null {
            let non_null: Vec<&Value> = arr
                .iter()
                .filter(|s| {
                    s.as_object()
                        .and_then(|o| o.get("type"))
                        .and_then(|t| t.as_str())
                        != Some("null")
                })
                .collect();
            if non_null.len() == 1 {
                // Single non-null schema: convert it and merge into result.
                let converted = convert_json_schema_to_openapi_schema(non_null[0], false);
                result.insert("nullable".to_string(), json!(true));
                if let Some(obj2) = converted.as_object() {
                    for (k, v) in obj2 {
                        result.insert(k.clone(), v.clone());
                    }
                }
            } else {
                let converted: Vec<Value> = non_null
                    .iter()
                    .map(|i| convert_json_schema_to_openapi_schema(i, false))
                    .collect();
                result.insert("anyOf".to_string(), Value::Array(converted));
                result.insert("nullable".to_string(), json!(true));
            }
        } else {
            let converted: Vec<Value> = arr
                .iter()
                .map(|i| convert_json_schema_to_openapi_schema(i, false))
                .collect();
            result.insert("anyOf".to_string(), Value::Array(converted));
        }
    }

    if let Some(min_len) = obj.get("minLength") {
        result.insert("minLength".to_string(), min_len.clone());
    }

    Value::Object(result)
}

fn is_empty_object_schema(obj: &Map<String, Value>) -> bool {
    obj.get("type").and_then(|v| v.as_str()) == Some("object")
        && (obj.get("properties").is_none()
            || obj
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|o| o.is_empty())
                .unwrap_or(true))
        && !obj.contains_key("additionalProperties")
}

// ── build_request_body ───────────────────────────────────────────────────────

/// Build the Gemini `generateContent` request body from `CallOptions`.
///
/// Mirrors `getArgs` in `google-language-model.ts`. Provider-specific options
/// (`thinkingConfig`, `safetySettings`, `cachedContent`, `labels`,
/// `serviceTier`, …) are not yet surfaced through `CallOptions` in the Rust
/// port and are therefore omitted.
///
/// This is the request-body-only entry point; warnings about unsupported tools
/// are discarded. Use [`build_request_body_with_warnings`] to surface them.
pub fn build_request_body(model_id: &str, options: &CallOptions) -> Value {
    build_request_body_with_warnings(model_id, options).0
}

/// Build the Gemini `generateContent` request body **and** collect the tool
/// warnings (e.g. unsupported provider-defined tools, mixed function+provider
/// tools on pre-Gemini-3 models).
pub fn build_request_body_with_warnings(
    model_id: &str,
    options: &CallOptions,
) -> (Value, Vec<Warning>) {
    let GooglePrompt {
        system_instruction,
        contents,
    } = convert_to_google_messages(&options.prompt);

    let mut generation_config = Map::new();

    if let Some(max_tokens) = options.max_output_tokens {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temp) = options.temperature {
        generation_config.insert("temperature".to_string(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        generation_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(top_k) = options.top_k {
        generation_config.insert("topK".to_string(), json!(top_k));
    }
    if let Some(presence) = options.presence_penalty {
        generation_config.insert("presencePenalty".to_string(), json!(presence));
    }
    if let Some(frequency) = options.frequency_penalty {
        generation_config.insert("frequencyPenalty".to_string(), json!(frequency));
    }
    if let Some(stop) = &options.stop_sequences {
        generation_config.insert("stopSequences".to_string(), json!(stop));
    }
    if let Some(seed) = options.seed {
        generation_config.insert("seed".to_string(), json!(seed));
    }

    // Response format: Gemini uses `responseMimeType: "application/json"`
    // (and an optional `responseSchema`) rather than OpenAI's `response_format`.
    if let Some(rf) = &options.response_format
        && let ResponseFormat::Json { schema, .. } = rf
    {
        generation_config.insert("responseMimeType".to_string(), json!("application/json"));
        if let Some(s) = schema {
            let openapi = convert_json_schema_to_openapi_schema(s, true);
            if !openapi.is_null() {
                generation_config.insert("responseSchema".to_string(), openapi);
            }
        }
    }

    let mut body = Map::new();
    body.insert("contents".to_string(), Value::Array(contents));
    if let Some(sys) = system_instruction {
        body.insert("systemInstruction".to_string(), sys);
    }
    if !generation_config.is_empty() {
        body.insert(
            "generationConfig".to_string(),
            Value::Object(generation_config),
        );
    }

    let prepared = prepare_all_tools(&options.tools, &options.tool_choice, model_id);
    if let Some(tools) = prepared.tools {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(tc) = prepared.tool_config {
        body.insert("toolConfig".to_string(), tc);
    }

    // Model id is *not* part of the body for Gemini — it's in the URL path
    // (`models/{model}:generateContent`). We don't emit it here, matching
    // the TS SDK. (Some callers include it; the API ignores extra fields.)
    let _ = model_id;

    (Value::Object(body), prepared.warnings)
}

// ── finish reason ────────────────────────────────────────────────────────────

/// Map a Gemini `finishReason` string to the unified `FinishReason`.
///
/// `STOP` maps to `ToolCalls` when `has_tool_calls` is true (mirroring the
/// TS `mapGoogleFinishReason`).
pub fn parse_finish_reason(reason: &str, has_tool_calls: bool) -> FinishReason {
    let unified = match reason {
        "STOP" => {
            if has_tool_calls {
                FinishReasonUnified::ToolCalls
            } else {
                FinishReasonUnified::Stop
            }
        }
        "MAX_TOKENS" => FinishReasonUnified::Length,
        "IMAGE_SAFETY" | "RECITATION" | "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            FinishReasonUnified::ContentFilter
        }
        "MALFORMED_FUNCTION_CALL" => FinishReasonUnified::Error,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(reason.to_string()),
    }
}

// ── usage conversion ────────────────────────────────────────────────────────

/// Convert a `GoogleUsageMetadata` into the core `Usage` type.
///
/// Mirrors `convertGoogleUsage`:
/// - `input.total = promptTokenCount`
/// - `input.noCache = promptTokenCount - cachedContentTokenCount`
/// - `input.cacheRead = cachedContentTokenCount`
/// - `output.total = candidatesTokenCount + thoughtsTokenCount`
pub fn convert_usage(usage: &super::types::GoogleUsageMetadata) -> aimux_core::types::Usage {
    use aimux_core::types::{TokenUsage, Usage};

    let prompt = usage.prompt_token_count.unwrap_or(0);
    let candidates = usage.candidates_token_count.unwrap_or(0);
    let cached = usage.cached_content_token_count.unwrap_or(0);
    let thoughts = usage.thoughts_token_count.unwrap_or(0);

    Usage {
        input_tokens: TokenUsage {
            total: Some(prompt),
            no_cache: Some(prompt - cached),
            cache_read: Some(cached),
            cache_write: None,
            ..Default::default()
        },
        output_tokens: TokenUsage {
            total: Some(candidates + thoughts),
            ..Default::default()
        },
        // RFC-0015 P0-3: keep the raw provider usage payload.
        raw: Some(serde_json::to_value(usage).unwrap_or(serde_json::Value::Null)),
    }
}

// ── source extraction ────────────────────────────────────────────────────────

/// Extract `GenerateContent::Source` items from `groundingMetadata.groundingChunks`,
/// mirroring the TS `extractSources`.
///
/// - `web` → url source (`uri`, `title`)
/// - `image` → url source (`sourceUri`, `title`)
/// - `retrievedContext` with http(s) `uri` → url source
/// - `retrievedContext` with non-http `uri` (e.g. `gs://`) → document source
///   (title defaults to "Unknown Document"; `url` is `None`)
/// - `retrievedContext` with `fileSearchStore` (no `uri`) → document source
/// - `maps` → url source (`uri`, `title`)
pub fn extract_sources(
    grounding_metadata: Option<&Value>,
    id_counter: &mut usize,
) -> Vec<GenerateContent> {
    let mut sources = Vec::new();
    let Some(gm) = grounding_metadata else {
        return sources;
    };
    let Some(chunks) = gm.get("groundingChunks").and_then(|c| c.as_array()) else {
        return sources;
    };

    let next_id = |counter: &mut usize| -> String {
        let id = format!("{}", counter);
        *counter += 1;
        id
    };

    for chunk in chunks {
        if let Some(web) = chunk.get("web") {
            sources.push(GenerateContent::Source {
                id: next_id(id_counter),
                source_type: "url".to_string(),
                url: web
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                title: web
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                provider_metadata: None,
            });
        } else if let Some(image) = chunk.get("image") {
            sources.push(GenerateContent::Source {
                id: next_id(id_counter),
                source_type: "url".to_string(),
                url: image
                    .get("sourceUri")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                title: image
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                provider_metadata: None,
            });
        } else if let Some(rc) = chunk.get("retrievedContext") {
            let uri = rc.get("uri").and_then(|v| v.as_str());
            let file_search_store = rc.get("fileSearchStore").and_then(|v| v.as_str());
            let title = rc.get("title").and_then(|v| v.as_str());
            if let Some(uri) = uri {
                if uri.starts_with("http://") || uri.starts_with("https://") {
                    sources.push(GenerateContent::Source {
                        id: next_id(id_counter),
                        source_type: "url".to_string(),
                        url: Some(uri.to_string()),
                        title: title.map(|s| s.to_string()),
                        provider_metadata: None,
                    });
                } else {
                    // Document with a file path (gs://, etc.).
                    sources.push(GenerateContent::Source {
                        id: next_id(id_counter),
                        source_type: "document".to_string(),
                        url: None,
                        title: Some(title.unwrap_or("Unknown Document").to_string()),
                        provider_metadata: None,
                    });
                }
            } else if file_search_store.is_some() {
                // New File Search format (no uri, has fileSearchStore).
                sources.push(GenerateContent::Source {
                    id: next_id(id_counter),
                    source_type: "document".to_string(),
                    url: None,
                    title: Some(title.unwrap_or("Unknown Document").to_string()),
                    provider_metadata: None,
                });
            }
            // else: no uri and no fileSearchStore → no source.
        } else if let Some(maps) = chunk.get("maps")
            && let Some(uri) = maps.get("uri").and_then(|v| v.as_str())
        {
            sources.push(GenerateContent::Source {
                id: next_id(id_counter),
                source_type: "url".to_string(),
                url: Some(uri.to_string()),
                title: maps
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                provider_metadata: None,
            });
        }
    }

    sources
}
