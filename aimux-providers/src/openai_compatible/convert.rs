//! Call options and prompt to the OpenAI-compatible chat-completions body.
//!
//! The Rust form of `getArgs` and `convertToOpenAICompatibleChatMessages` of
//! `@ai-sdk/openai-compatible`. The body is built in the AI SDK's order (model,
//! `user`, sampling settings, `response_format`, `stop`, `seed`, the
//! pass-through fields of the provider's own options namespace, then
//! `reasoning_effort`, `verbosity`, `messages`, `tools`, `tool_choice`).
//! Compatible endpoint capabilities arrive as [`ChatDialect`] data.

use base64::Engine;
use serde_json::{Map, Value, json};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::tool::Tool;
use aimux_core::types::{FinishReason, FinishReasonUnified, Warning};

use super::config::ChatDialect;

/// Option keys of the generic compatible schema
/// (`openaiCompatibleLanguageModelChatOptions`): consumed by the model, never
/// passed through.
const SCHEMA_KEYS: [&str; 4] = [
    "user",
    "reasoningEffort",
    "textVerbosity",
    "strictJsonSchema",
];

/// `tools` / `tool_choice` ready for the body, with the warnings raised while
/// preparing them.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreparedTools {
    pub tools: Option<Vec<Value>>,
    pub tool_choice: Option<Value>,
    pub warnings: Vec<Warning>,
}

/// The wire form of a function tool.
pub(crate) fn function_tool_value(tool: &aimux_core::tool::FunctionTool) -> Value {
    let mut function = Map::new();
    function.insert("name".into(), json!(tool.name));
    if let Some(description) = &tool.description {
        function.insert("description".into(), json!(description));
    }
    function.insert("parameters".into(), tool.input_schema.clone());
    if let Some(strict) = tool.strict {
        function.insert("strict".into(), json!(strict));
    }
    json!({ "type": "function", "function": function })
}

/// The wire form of the tool choice.
pub(crate) fn tool_choice_value(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Tool { tool_name } => {
            json!({ "type": "function", "function": { "name": tool_name } })
        }
    }
}

/// Function tools only; a provider-defined tool is unsupported by the generic
/// endpoint and warned about (the AI SDK's `prepareTools`).
pub(crate) fn prepare_function_tools(options: &CallOptions) -> PreparedTools {
    let mut prepared = PreparedTools::default();
    let Some(tools) = options.tools.as_ref().filter(|tools| !tools.is_empty()) else {
        return prepared;
    };
    let mut wire = Vec::new();
    for tool in tools {
        match tool {
            Tool::Function(function) => wire.push(function_tool_value(function)),
            Tool::Provider(provider) => prepared.warnings.push(Warning::Unsupported {
                feature: format!("provider-defined tool {}", provider.id),
                details: None,
            }),
        }
    }
    if !wire.is_empty() {
        prepared.tools = Some(wire);
        prepared.tool_choice = Some(tool_choice_value(&options.tool_choice));
    }
    prepared
}

/// `snake_case` / `kebab-case` provider name to `camelCase` (the AI SDK's
/// `toCamelCase`).
#[must_use]
pub(crate) fn to_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        match chars.peek() {
            Some(next) if (ch == '_' || ch == '-') && next.is_ascii_lowercase() => {
                out.push(next.to_ascii_uppercase());
                chars.next();
            }
            _ => out.push(ch),
        }
    }
    out
}

/// What the converter needs to know about the provider it builds for.
pub(crate) struct ChatBodySpec<'a> {
    /// First segment of the provider string: the providerOptions namespace.
    pub provider_options_name: &'a str,
    pub include_usage: bool,
    pub supports_structured_outputs: bool,
    pub supports_multi_part_tool_content: bool,
    pub dialect: &'a ChatDialect,
}

/// A request body and what building it produced.
#[derive(Debug, Clone)]
pub struct RequestBodyResult {
    /// The JSON body, before the provider's `transform_request_body`.
    pub body: Value,
    /// Warnings raised while building it.
    pub warnings: Vec<Warning>,
    /// The key provider metadata is reported under.
    pub metadata_key: String,
}

fn namespace<'a>(options: &'a CallOptions, key: &str) -> Option<&'a Map<String, Value>> {
    options
        .provider_options
        .as_ref()
        .and_then(|all| all.get(key))
        .and_then(Value::as_object)
}

/// The key provider metadata is reported under (`resolveProviderOptionsKey`):
/// the provider name, unless the call used only its camelCase form.
fn resolve_metadata_key(name: &str, options: &CallOptions) -> String {
    let camel = to_camel_case(name);
    if camel != name && namespace(options, name).is_none() && namespace(options, &camel).is_some() {
        camel
    } else {
        name.to_string()
    }
}

fn option_string(
    merged: &Map<String, Value>,
    namespace_name: &str,
    key: &str,
) -> Result<Option<String>, AiMuxError> {
    match merged.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(AiMuxError::InvalidArgument(format!(
            "invalid provider options for \"{namespace_name}\": `{key}` must be a string"
        ))),
    }
}

/// Build the request body (`getArgs`).
///
/// # Errors
///
/// `InvalidArgument` for a malformed provider option or a prompt part that has
/// no wire form.
pub(crate) fn build_request_body(
    model_id: &str,
    options: &CallOptions,
    stream: bool,
    spec: &ChatBodySpec<'_>,
) -> Result<RequestBodyResult, AiMuxError> {
    let name = spec.provider_options_name;
    let camel = to_camel_case(name);
    let dialect = spec.dialect;
    let mut warnings = Vec::new();

    // Resolved provider options: the generic namespace, then the provider's
    // own, then its camelCase form; later wins per key.
    let mut merged = Map::new();
    if let Some(deprecated) = namespace(options, "openai-compatible") {
        warnings.push(Warning::Deprecated {
            setting: "providerOptions key 'openai-compatible'".to_string(),
            message: "Use 'openaiCompatible' instead.".to_string(),
        });
        merged.extend(deprecated.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    for key in ["openaiCompatible", name, camel.as_str()] {
        if let Some(object) = namespace(options, key) {
            merged.extend(object.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    if camel != name && namespace(options, name).is_some() {
        warnings.push(Warning::Deprecated {
            setting: format!("providerOptions key '{name}'"),
            message: format!("Use '{camel}' instead."),
        });
    }

    let user = option_string(&merged, name, "user")?;
    let reasoning_effort = option_string(&merged, name, "reasoningEffort")?.or_else(|| {
        options
            .reasoning
            .filter(|effort| effort.is_custom())
            .map(|effort| effort.to_string())
    });
    let verbosity = option_string(&merged, name, "textVerbosity")?;
    let strict_json_schema = merged
        .get("strictJsonSchema")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let metadata_key = resolve_metadata_key(name, options);
    let mut body = Map::new();
    body.insert("model".into(), json!(model_id));
    if let Some(user) = user {
        body.insert("user".into(), json!(user));
    }
    if let Some(max_tokens) = options.max_output_tokens {
        body.insert(
            dialect.max_tokens_key.unwrap_or("max_tokens").into(),
            json!(max_tokens),
        );
    }
    for (key, value) in [
        ("temperature", options.temperature),
        ("top_p", options.top_p),
        ("frequency_penalty", options.frequency_penalty),
        ("presence_penalty", options.presence_penalty),
    ] {
        if let Some(value) = value {
            body.insert(key.into(), json!(value));
        }
    }
    if let Some(top_k) = options.top_k {
        if dialect.supports_top_k {
            body.insert("top_k".into(), json!(top_k));
        } else {
            warnings.push(Warning::Unsupported {
                feature: "topK".to_string(),
                details: None,
            });
        }
    }

    match &options.response_format {
        Some(ResponseFormat::Json {
            schema,
            name: format_name,
            description,
        }) if dialect.supports_response_format => match schema {
            Some(schema) if spec.supports_structured_outputs => {
                let mut json_schema = Map::new();
                json_schema.insert("schema".into(), schema.clone());
                json_schema.insert("strict".into(), json!(strict_json_schema));
                json_schema.insert(
                    "name".into(),
                    json!(format_name.clone().unwrap_or_else(|| "response".into())),
                );
                if let Some(description) = description {
                    json_schema.insert("description".into(), json!(description));
                }
                body.insert(
                    "response_format".into(),
                    json!({ "type": "json_schema", "json_schema": json_schema }),
                );
            }
            Some(_) => {
                warnings.push(Warning::Unsupported {
                    feature: "responseFormat".to_string(),
                    details: Some(
                        "JSON response format schema is only supported with structuredOutputs"
                            .to_string(),
                    ),
                });
                body.insert("response_format".into(), json!({ "type": "json_object" }));
            }
            None => {
                body.insert("response_format".into(), json!({ "type": "json_object" }));
            }
        },
        Some(ResponseFormat::Json { .. }) => warnings.push(Warning::Unsupported {
            feature: "responseFormat".to_string(),
            details: Some("response_format is not supported by this provider".to_string()),
        }),
        Some(ResponseFormat::Text) | None => {}
    }

    if let Some(stop) = &options.stop_sequences {
        body.insert("stop".into(), json!(stop));
    }
    if let Some(seed) = options.seed {
        body.insert("seed".into(), json!(seed));
    }

    // Fields of the provider's own namespace that the schema does not know go
    // to the body as given (the generic `openaiCompatible` namespace is
    // schema-only: unknown fields there are dropped).
    for key in [name, camel.as_str()] {
        if let Some(object) = namespace(options, key) {
            for (field, value) in object {
                if !SCHEMA_KEYS.contains(&field.as_str()) {
                    body.insert(field.clone(), value.clone());
                }
            }
        }
    }

    if let Some(effort) = reasoning_effort {
        body.insert("reasoning_effort".into(), json!(effort));
    }
    if let Some(verbosity) = verbosity {
        body.insert("verbosity".into(), json!(verbosity));
    }

    let messages = convert_messages(
        &options.prompt,
        &MessageSpec {
            metadata_key: &metadata_key,
            supports_multi_part_tool_content: spec.supports_multi_part_tool_content,
        },
    )?;
    body.insert("messages".into(), Value::Array(messages));

    if dialect.supports_tools {
        let prepared = prepare_function_tools(options);
        warnings.extend(prepared.warnings);
        if let Some(tools) = prepared.tools {
            body.insert("tools".into(), Value::Array(tools));
        }
        if let Some(choice) = prepared.tool_choice {
            body.insert("tool_choice".into(), choice);
        }
    } else if options
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
    {
        warnings.push(Warning::Unsupported {
            feature: "tools".to_string(),
            details: Some("tools are not supported by this provider".to_string()),
        });
    }

    if stream {
        body.insert("stream".into(), json!(true));
        if spec.include_usage {
            body.insert("stream_options".into(), json!({ "include_usage": true }));
        }
    }

    Ok(RequestBodyResult {
        body: Value::Object(body),
        warnings,
        metadata_key,
    })
}

// ── Messages ────────────────────────────────────────────────────────────────

pub(crate) struct MessageSpec<'a> {
    pub metadata_key: &'a str,
    pub supports_multi_part_tool_content: bool,
}

/// The provider options of a message or part that belong on the wire object
/// (`getOpenAIMetadata`): the generic namespace, then the provider's own.
fn wire_metadata(provider_options: Option<&Value>, key: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for ns in ["openaiCompatible", key] {
        if let Some(object) = provider_options
            .and_then(|options| options.get(ns))
            .and_then(Value::as_object)
        {
            out.extend(object.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    out
}

fn with_metadata(mut object: Value, metadata: Map<String, Value>) -> Value {
    if let Some(map) = object.as_object_mut() {
        map.extend(metadata);
    }
    object
}

fn part_options(part: &ContentPart) -> Option<&Value> {
    match part {
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
        }
        | ContentPart::FileUrl {
            provider_options, ..
        }
        | ContentPart::FileReference {
            provider_options, ..
        }
        | ContentPart::Reasoning {
            provider_options, ..
        }
        | ContentPart::ToolCall {
            provider_options, ..
        }
        | ContentPart::ToolResult {
            provider_options, ..
        } => provider_options.as_ref(),
    }
}

/// `messages` for the body.
///
/// # Errors
///
/// `InvalidArgument` when a part has no wire form (an unsupported media type,
/// an audio or PDF part given by URL).
pub(crate) fn convert_messages(
    prompt: &LanguageModelPrompt,
    spec: &MessageSpec<'_>,
) -> Result<Vec<Value>, AiMuxError> {
    let mut out = Vec::new();
    for message in prompt {
        out.extend(convert_message(message, spec)?);
    }
    Ok(out)
}

fn convert_message(
    message: &LanguageModelPromptMessage,
    spec: &MessageSpec<'_>,
) -> Result<Vec<Value>, AiMuxError> {
    let key = spec.metadata_key;
    let message_metadata = wire_metadata(message.provider_options.as_ref(), key);
    match message.role {
        Role::System => {
            let text: String = message
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let mut metadata = message_metadata;
            if let Some(first) = message.content.first() {
                metadata.extend(wire_metadata(part_options(first), key));
            }
            Ok(vec![with_metadata(
                json!({ "role": "system", "content": text }),
                metadata,
            )])
        }
        Role::User => Ok(vec![convert_user_message(message, spec, message_metadata)?]),
        Role::Assistant => Ok(vec![convert_assistant_message(
            message,
            spec,
            message_metadata,
        )]),
        Role::Tool => Ok(message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolResult {
                    tool_call_id,
                    result,
                    provider_options,
                    ..
                } => Some(with_metadata(
                    json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": tool_result_content(result, spec.supports_multi_part_tool_content),
                    }),
                    wire_metadata(provider_options.as_ref(), key),
                )),
                _ => None,
            })
            .collect()),
    }
}

/// A tool result as message content: text, except that a vendor that accepts
/// structured tool-result content (`supports_multi_part_tool_content`) gets an
/// array result as the content parts it is.
fn tool_result_content(result: &Value, multi_part: bool) -> Value {
    match result {
        Value::String(text) => Value::String(text.clone()),
        Value::Array(_) if multi_part => result.clone(),
        other => Value::String(other.to_string()),
    }
}

fn convert_user_message(
    message: &LanguageModelPromptMessage,
    spec: &MessageSpec<'_>,
    message_metadata: Map<String, Value>,
) -> Result<Value, AiMuxError> {
    let key = spec.metadata_key;
    if let [
        ContentPart::Text {
            text,
            provider_options,
        },
    ] = message.content.as_slice()
    {
        let mut metadata = message_metadata;
        metadata.extend(wire_metadata(provider_options.as_ref(), key));
        return Ok(with_metadata(
            json!({ "role": "user", "content": text }),
            metadata,
        ));
    }
    let mut parts = Vec::new();
    for (index, part) in message.content.iter().enumerate() {
        let wire = convert_user_part(part, index, key)?;
        if !wire.is_null() {
            parts.push(wire);
        }
    }
    Ok(with_metadata(
        json!({ "role": "user", "content": parts }),
        message_metadata,
    ))
}

fn convert_user_part(part: &ContentPart, index: usize, key: &str) -> Result<Value, AiMuxError> {
    let metadata = wire_metadata(part_options(part), key);
    let wire = match part {
        ContentPart::Text { text, .. } => json!({ "type": "text", "text": text }),
        ContentPart::Image {
            image, media_type, ..
        } => file_part(
            media_type,
            FileSource::Base64(&base64::engine::general_purpose::STANDARD.encode(image)),
            None,
            index,
        )?,
        ContentPart::File {
            data,
            media_type,
            filename,
            ..
        } => file_part(
            media_type,
            FileSource::Base64(&base64::engine::general_purpose::STANDARD.encode(data)),
            filename.as_deref(),
            index,
        )?,
        ContentPart::FileBase64 {
            data,
            media_type,
            filename,
            ..
        } => file_part(
            media_type,
            FileSource::Base64(data),
            filename.as_deref(),
            index,
        )?,
        ContentPart::FileUrl {
            url, media_type, ..
        } => file_part(media_type, FileSource::Url(url), None, index)?,
        ContentPart::FileReference {
            media_type,
            reference,
            filename,
            ..
        } => file_part(
            media_type,
            FileSource::Reference(reference, key),
            filename.as_deref(),
            index,
        )?,
        // Reasoning and tool traffic have no user-content form.
        ContentPart::Reasoning { .. }
        | ContentPart::ToolCall { .. }
        | ContentPart::ToolResult { .. } => return Ok(Value::Null),
    };
    Ok(with_metadata(wire, metadata))
}

enum FileSource<'a> {
    Base64(&'a str),
    Url(&'a str),
    /// A provider file reference object and the provider key to look up.
    Reference(&'a Value, &'a str),
}

fn top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}

/// Detect the image type of base64 data for a bare or wildcard `image` type.
fn resolve_image_media_type(media_type: &str, b64: &str) -> String {
    if media_type != "image" && !media_type.ends_with("/*") {
        return media_type.to_string();
    }
    if b64.starts_with("iVBORw0KGgo") {
        "image/png"
    } else if b64.starts_with("/9j/") {
        "image/jpeg"
    } else if b64.starts_with("R0lGOD") {
        "image/gif"
    } else if b64.starts_with("UklGR") {
        "image/webp"
    } else {
        "image/png"
    }
    .to_string()
}

fn file_part(
    media_type: &str,
    source: FileSource<'_>,
    filename: Option<&str>,
    index: usize,
) -> Result<Value, AiMuxError> {
    let unsupported = |what: String| AiMuxError::InvalidArgument(what);
    if let FileSource::Reference(reference, key) = source {
        let file_id = match reference
            .get(key)
            .or_else(|| reference.get("openaiCompatible"))
        {
            Some(Value::String(id)) => id.clone(),
            Some(other) => other.to_string(),
            None => {
                let available: Vec<&str> = reference
                    .as_object()
                    .map(|m| m.keys().map(String::as_str).collect())
                    .unwrap_or_default();
                return Err(unsupported(format!(
                    "No provider reference found for provider '{key}'. Available providers: {}",
                    available.join(", ")
                )));
            }
        };
        return Ok(json!({ "type": "file", "file": { "file_id": file_id } }));
    }

    match top_level_media_type(media_type) {
        "image" => {
            let url = match source {
                FileSource::Url(url) => url.to_string(),
                FileSource::Base64(b64) => format!(
                    "data:{};base64,{}",
                    resolve_image_media_type(media_type, b64),
                    b64
                ),
                FileSource::Reference(..) => unreachable!("handled above"),
            };
            Ok(json!({ "type": "image_url", "image_url": { "url": url } }))
        }
        "audio" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("audio file parts with URLs".to_string()));
            };
            let format = match media_type {
                "audio/wav" => "wav",
                "audio/mp3" | "audio/mpeg" => "mp3",
                other => {
                    return Err(unsupported(format!(
                        "audio content parts with media type {other}"
                    )));
                }
            };
            Ok(json!({ "type": "input_audio", "input_audio": { "data": b64, "format": format } }))
        }
        _ if media_type == "application/pdf" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("PDF file parts with URLs".to_string()));
            };
            let filename = filename
                .map(str::to_string)
                .unwrap_or_else(|| format!("part-{index}.pdf"));
            Ok(json!({
                "type": "file",
                "file": {
                    "filename": filename,
                    "file_data": format!("data:application/pdf;base64,{b64}"),
                }
            }))
        }
        _ if top_level_media_type(media_type) == "text" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("text file parts with URLs".to_string()));
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| unsupported(format!("text file part is not valid base64: {e}")))?;
            let text = String::from_utf8(bytes)
                .map_err(|_| unsupported("text file part is not valid UTF-8".to_string()))?;
            Ok(json!({ "type": "text", "text": text }))
        }
        _ => Err(unsupported(format!("file part media type {media_type}"))),
    }
}

fn convert_assistant_message(
    message: &LanguageModelPromptMessage,
    spec: &MessageSpec<'_>,
    message_metadata: Map<String, Value>,
) -> Value {
    let key = spec.metadata_key;
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut metadata = message_metadata;
    for part in &message.content {
        match part {
            ContentPart::Text { .. } => {
                if let ContentPart::Text { text: t, .. } = part {
                    text.push_str(t);
                }
                metadata.extend(wire_metadata(part_options(part), key));
            }
            ContentPart::Reasoning { text: t, .. } => reasoning.push_str(t),
            ContentPart::ToolCall {
                tool_call_id,
                tool_name,
                input,
                thought_signature,
                provider_options,
                ..
            } => {
                let arguments = if input.is_null() {
                    "{}".to_string()
                } else {
                    input.to_string()
                };
                let mut call = json!({
                    "id": tool_call_id,
                    "type": "function",
                    "function": { "name": tool_name, "arguments": arguments },
                });
                if let Some(signature) = thought_signature.as_ref().filter(|s| !s.is_empty()) {
                    call["extra_content"] = json!({ "google": { "thought_signature": signature } });
                }
                tool_calls.push(with_metadata(
                    call,
                    wire_metadata(provider_options.as_ref(), key),
                ));
            }
            // Provider-executed results and media have no assistant-content
            // form on this wire.
            _ => {}
        }
    }
    let mut wire = Map::new();
    wire.insert("role".into(), json!("assistant"));
    wire.insert(
        "content".into(),
        if tool_calls.is_empty() || !text.is_empty() {
            json!(text)
        } else {
            Value::Null
        },
    );
    if !reasoning.is_empty() {
        wire.insert("reasoning_content".into(), json!(reasoning));
    }
    if !tool_calls.is_empty() {
        wire.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    wire.extend(metadata);
    Value::Object(wire)
}

/// Map an OpenAI finish reason string to the unified one, keeping the raw.
#[must_use]
pub(crate) fn parse_finish_reason(raw: &str) -> FinishReason {
    let unified = match raw {
        "stop" => FinishReasonUnified::Stop,
        "length" => FinishReasonUnified::Length,
        "tool_calls" | "function_call" => FinishReasonUnified::ToolCalls,
        "content_filter" => FinishReasonUnified::ContentFilter,
        _ => FinishReasonUnified::Other,
    };
    FinishReason {
        unified,
        raw: Some(raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_follows_the_ai_sdk() {
        assert_eq!(to_camel_case("acme"), "acme");
        assert_eq!(to_camel_case("my_provider"), "myProvider");
        assert_eq!(to_camel_case("my-provider"), "myProvider");
        assert_eq!(
            to_camel_case("vertex_ai_openai_models"),
            "vertexAiOpenaiModels"
        );
        assert_eq!(to_camel_case("a_1"), "a_1");
    }
}
