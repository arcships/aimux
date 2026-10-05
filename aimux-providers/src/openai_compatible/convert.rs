//! Call options and prompt to the OpenAI-compatible chat-completions body.
//!
//! The Rust form of `getArgs` and `convertToOpenAICompatibleChatMessages` of
//! `@ai-sdk/openai-compatible`. The body is built in the AI SDK's order (model,
//! `user`, sampling settings, `response_format`, `stop`, `seed`, the
//! pass-through fields of the provider's own options namespace, then
//! `reasoning_effort`, `verbosity`, `messages`, `tools`, `tool_choice`).
//! Compatible endpoint capabilities arrive as [`ChatDialect`] data.

use aimux_core::shared::SharedProviderOptions;
use base64::Engine;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::language_model_message::{
    AssistantPart, FilePart, LanguageModelMessage, LanguageModelPrompt, ReasoningPart, TextPart,
    ToolCallPart, ToolPart, ToolResultPart, UserPart,
};
use aimux_core::options::{CallOptions, ResponseFormat, ToolChoice};
use aimux_core::shared::{FileBytes, FileData};
use aimux_core::tool::Tool;
use aimux_core::types::{FinishReason, FinishReasonUnified, Warning};
use aimux_provider_utils::{get_top_level_media_type, resolve_full_media_type};

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
    prepared.tools = Some(wire);
    prepared.tool_choice = options.tool_choice.as_ref().map(tool_choice_value);
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
}

/// The key provider metadata is reported under (`resolveProviderOptionsKey`):
/// the camelCase name when supplied, otherwise the raw provider name.
fn resolve_metadata_key(name: &str, options: &CallOptions) -> String {
    let camel = to_camel_case(name);
    if camel != name && namespace(options, &camel).is_some() {
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
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(AiMuxError::InvalidArgument(format!(
            "invalid provider options for \"{namespace_name}\": `{key}` must be a string"
        ))),
    }
}

fn validate_options(options: &Map<String, Value>, namespace_name: &str) -> Result<(), AiMuxError> {
    for key in ["user", "reasoningEffort", "textVerbosity"] {
        option_string(options, namespace_name, key)?;
    }
    if options
        .get("strictJsonSchema")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(AiMuxError::InvalidArgument(format!(
            "invalid provider options for \"{namespace_name}\": `strictJsonSchema` must be a boolean"
        )));
    }
    Ok(())
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
        validate_options(deprecated, "openai-compatible")?;
        warnings.push(Warning::Deprecated {
            setting: "providerOptions key 'openai-compatible'".to_string(),
            message: "Use 'openaiCompatible' instead.".to_string(),
        });
        merged.extend(deprecated.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    for key in ["openaiCompatible", name, camel.as_str()] {
        if let Some(object) = namespace(options, key) {
            validate_options(object, key)?;
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

/// The generic `openaiCompatible` metadata of a message or part
/// (`getOpenAIMetadata`).
fn wire_metadata(provider_options: Option<&SharedProviderOptions>) -> Map<String, Value> {
    provider_options
        .and_then(|options| options.get("openaiCompatible"))
        .cloned()
        .unwrap_or_default()
}

fn with_metadata(mut object: Value, metadata: Map<String, Value>) -> Value {
    if let Some(map) = object.as_object_mut() {
        map.extend(metadata);
    }
    object
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
    message: &LanguageModelMessage,
    spec: &MessageSpec<'_>,
) -> Result<Vec<Value>, AiMuxError> {
    match message {
        LanguageModelMessage::System { content, provider_options } => Ok(vec![with_metadata(
            json!({ "role": "system", "content": content }),
            wire_metadata(provider_options.as_ref()),
        )]),
        LanguageModelMessage::User { content, provider_options } => Ok(vec![convert_user_message(
            content,
            wire_metadata(provider_options.as_ref()),
        )?]),
        LanguageModelMessage::Assistant { content, provider_options } => Ok(vec![convert_assistant_message(
            content,
            spec,
            wire_metadata(provider_options.as_ref()),
        )]),
        LanguageModelMessage::Tool { content, .. } => Ok(content
            .iter()
            .map(|part| {
                let ToolPart::ToolResult(ToolResultPart {
                    tool_call_id,
                    result,
                    provider_options,
                    ..
                }) = part;
                with_metadata(
                    json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": tool_result_content(result, spec.supports_multi_part_tool_content),
                    }),
                    wire_metadata(provider_options.as_ref()),
                )
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
    content: &[UserPart],
    message_metadata: Map<String, Value>,
) -> Result<Value, AiMuxError> {
    if let [
        UserPart::Text(TextPart {
            text,
            provider_options,
        }),
    ] = content
    {
        let metadata = wire_metadata(provider_options.as_ref());
        return Ok(with_metadata(
            json!({ "role": "user", "content": text }),
            metadata,
        ));
    }
    let parts = content
        .iter()
        .map(convert_user_part)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(with_metadata(
        json!({ "role": "user", "content": parts }),
        message_metadata,
    ))
}

fn convert_user_part(part: &UserPart) -> Result<Value, AiMuxError> {
    let (wire, provider_options) = match part {
        UserPart::Text(TextPart {
            text,
            provider_options,
        }) => (json!({ "type": "text", "text": text }), provider_options),
        UserPart::File(file) => {
            let wire = match &file.data {
                FileData::Data {
                    data: FileBytes::Binary(data),
                } => file_part(
                    file,
                    FileSource::Base64(&base64::engine::general_purpose::STANDARD.encode(data)),
                )?,
                FileData::Data {
                    data: FileBytes::Base64(data),
                } => file_part(file, FileSource::Base64(data))?,
                FileData::Url { url, .. } => file_part(file, FileSource::Url(url))?,
                FileData::Reference { .. } => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "file parts with provider references".to_string(),
                    ));
                }
                FileData::Text { .. } => {
                    return Err(AiMuxError::UnsupportedFunctionality(
                        "text file parts".to_string(),
                    ));
                }
            };
            (wire, &file.provider_options)
        }
    };
    Ok(with_metadata(
        wire,
        wire_metadata(provider_options.as_ref()),
    ))
}

enum FileSource<'a> {
    Base64(&'a str),
    Url(&'a str),
}

fn file_part(part: &FilePart, source: FileSource<'_>) -> Result<Value, AiMuxError> {
    let media_type = part.media_type.as_str();
    let filename = part.filename.as_deref();
    let unsupported = |what: String| AiMuxError::UnsupportedFunctionality(what);

    match get_top_level_media_type(media_type) {
        kind @ ("image" | "video") => {
            let url = match source {
                FileSource::Url(url) => url.to_string(),
                FileSource::Base64(b64) => {
                    format!("data:{};base64,{}", resolve_full_media_type(part)?, b64)
                }
            };
            let kind = format!("{kind}_url");
            Ok(json!({ "type": kind, kind: { "url": url } }))
        }
        "audio" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("audio file parts with URLs".to_string()));
            };
            let full_media_type = resolve_full_media_type(part)?;
            let format = match full_media_type.as_str() {
                "audio/wav" => "wav",
                "audio/mp3" | "audio/mpeg" => "mp3",
                other => {
                    return Err(unsupported(format!("audio media type {other}")));
                }
            };
            Ok(json!({ "type": "input_audio", "input_audio": { "data": b64, "format": format } }))
        }
        "application" => {
            let FileSource::Base64(b64) = source else {
                return Err(unsupported("PDF file parts with URLs".to_string()));
            };
            let full_media_type = resolve_full_media_type(part)?;
            if full_media_type != "application/pdf" {
                return Err(unsupported(format!(
                    "file part media type {full_media_type}"
                )));
            }
            let filename = filename.unwrap_or("document.pdf");
            Ok(json!({
                "type": "file",
                "file": {
                    "filename": filename,
                    "file_data": format!("data:application/pdf;base64,{b64}"),
                }
            }))
        }
        "text" => {
            let b64 = match source {
                FileSource::Url(url) => return Ok(json!({ "type": "text", "text": url })),
                FileSource::Base64(b64) => b64,
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| unsupported(format!("text file part is not valid base64: {e}")))?;
            let text = String::from_utf8_lossy(&bytes);
            Ok(json!({ "type": "text", "text": text }))
        }
        _ => Err(unsupported(format!("file part media type {media_type}"))),
    }
}

fn convert_assistant_message(
    content: &[AssistantPart],
    spec: &MessageSpec<'_>,
    message_metadata: Map<String, Value>,
) -> Value {
    let key = spec.metadata_key;
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let metadata = message_metadata;
    for part in content {
        match part {
            AssistantPart::Text(TextPart { text: t, .. }) => text.push_str(t),
            AssistantPart::Reasoning(ReasoningPart { text: t, .. }) => reasoning.push_str(t),
            AssistantPart::ToolCall(ToolCallPart {
                tool_call_id,
                tool_name,
                input,
                thought_signature,
                provider_options,
                ..
            }) => {
                let arguments = input.to_string();
                let mut call = json!({
                    "id": tool_call_id,
                    "type": "function",
                    "function": { "name": tool_name, "arguments": arguments },
                });
                call = with_metadata(call, wire_metadata(provider_options.as_ref()));
                let signature = provider_options.as_ref().and_then(|options| {
                    options
                        .get(key)
                        .and_then(|options| options.get("thoughtSignature"))
                        .or_else(|| {
                            options
                                .get("google")
                                .and_then(|options| options.get("thoughtSignature"))
                        })
                });
                let signature = signature
                    .and_then(|value| match value {
                        Value::Null | Value::Bool(false) => None,
                        Value::String(text) if text.is_empty() => None,
                        Value::Number(number) if number.as_f64() == Some(0.0) => None,
                        Value::String(text) => Some(text.clone()),
                        other => Some(other.to_string()),
                    })
                    .or_else(|| thought_signature.clone().filter(|s| !s.is_empty()));
                if let Some(signature) = signature {
                    call["extra_content"] = json!({ "google": { "thought_signature": signature } });
                }
                tool_calls.push(call);
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
