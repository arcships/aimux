//! The providerOptions of the DeepSeek chat model
//! (`deepseek-chat-language-model-options.ts`, `deepseek-file-part-options.ts`).
//!
//! They are read from the namespace named by the provider (`deepseek`).

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use aimux_core::error::AiMuxError;

/// `thinking` of the call options. `adaptive` is accepted for backwards
/// compatibility and mapped to `enabled`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeepSeekThinkingOptions {
    #[serde(rename = "type", default, deserialize_with = "deserialize_optional")]
    pub kind: Option<ThinkingType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThinkingType {
    Adaptive,
    Enabled,
    Disabled,
}

/// `reasoningEffort` of the call options. `medium` and `xhigh` are accepted for
/// backwards compatibility and mapped to canonical DeepSeek values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProviderReasoningEffort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// `providerOptions.<provider>` of a call (`deepseekLanguageModelChatOptions`).
/// Fields the schema does not know are dropped.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeepSeekChatOptions {
    /// Whether to return log probabilities for generated tokens.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub logprobs: Option<bool>,
    /// Number of most likely tokens to return at each token position (0 to
    /// 20); setting it enables `logprobs`.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub top_logprobs: Option<u8>,
    /// An opaque identifier for the end user: ASCII letters, numbers,
    /// underscores and hyphens, at most 512 characters.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub user_id: Option<String>,
    /// The thinking configuration.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub thinking: Option<DeepSeekThinkingOptions>,
    /// The thinking strength.
    #[serde(default, deserialize_with = "deserialize_optional")]
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    #[serde(
        rename = "strictJsonSchema",
        default,
        deserialize_with = "deserialize_optional"
    )]
    pub _strict_json_schema: Option<bool>,
}

/// The `providerOptions` of a message (`name`, and for an assistant message
/// `prefix`).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct DeepSeekMessageOptions {
    /// The name of the participant represented by the message.
    pub name: Option<String>,
    /// Whether the assistant message is a prefix DeepSeek should continue.
    pub prefix: Option<bool>,
}

/// The `providerOptions` of a file part.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeepSeekFilePartOptions {
    /// How DeepSeek processes an image sent as an `image_url` part
    /// (`low`, `high`, `original`, `auto`).
    pub image_detail: Option<String>,
    /// Send inline image data as a `file` part instead of an `image_url`.
    pub file_data: Option<bool>,
}

// An optional schema field may be absent, but an explicit null is invalid.
fn deserialize_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// The options of `namespace` in a providerOptions value; no namespace is the
/// defaults.
fn parse_namespace<T: Default + for<'de> Deserialize<'de>>(
    namespace: Option<&Value>,
    name: &str,
) -> Result<T, AiMuxError> {
    match namespace.filter(|options| !options.is_null()) {
        None => Ok(T::default()),
        Some(options) if !options.is_object() => Err(invalid(name, "expected an object")),
        Some(options) => serde_json::from_value(options.clone()).map_err(|error| {
            AiMuxError::InvalidArgument(format!("invalid provider options for \"{name}\": {error}"))
        }),
    }
}

fn invalid(name: &str, message: &str) -> AiMuxError {
    AiMuxError::InvalidArgument(format!(
        "invalid provider options for \"{name}\": {message}"
    ))
}

/// The chat options of a call.
///
/// # Errors
///
/// `InvalidArgument` when an option has the wrong type or is out of range.
pub(crate) fn parse_chat_options(
    provider_options: Option<&HashMap<String, Value>>,
    name: &str,
) -> Result<DeepSeekChatOptions, AiMuxError> {
    let namespace = provider_options.and_then(|all| all.get(name));
    if namespace
        .and_then(|options| options.get("thinking"))
        .is_some_and(|thinking| !thinking.is_object())
    {
        return Err(invalid(name, "thinking must be an object"));
    }
    let options: DeepSeekChatOptions = parse_namespace(namespace, name)?;
    if options.top_logprobs.is_some_and(|n| n > 20) {
        return Err(invalid(name, "topLogprobs must be at most 20"));
    }
    if let Some(user_id) = &options.user_id {
        if user_id.is_empty()
            || !user_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid(name, "userId must match /^[a-zA-Z0-9_-]+$/"));
        }
        if user_id.len() > 512 {
            return Err(invalid(name, "userId must be at most 512 characters long"));
        }
    }
    Ok(options)
}

/// The options of a message.
///
/// # Errors
///
/// `InvalidArgument` when `name` is not a string or `prefix` is not `true`.
pub(crate) fn parse_message_options(
    provider_options: Option<&Value>,
    name: &str,
) -> Result<DeepSeekMessageOptions, AiMuxError> {
    let options: DeepSeekMessageOptions =
        parse_namespace(provider_options.and_then(|all| all.get(name)), name)?;
    if options.prefix == Some(false) {
        return Err(invalid(name, "prefix must be true"));
    }
    Ok(options)
}

/// The options of a file part.
///
/// # Errors
///
/// `InvalidArgument` when `imageDetail` is not a known value or `fileData` is
/// not `true`.
pub(crate) fn parse_file_part_options(
    provider_options: Option<&Value>,
    name: &str,
) -> Result<DeepSeekFilePartOptions, AiMuxError> {
    let options: DeepSeekFilePartOptions =
        parse_namespace(provider_options.and_then(|all| all.get(name)), name)?;
    if options.file_data == Some(false) {
        return Err(invalid(name, "fileData must be true"));
    }
    if options
        .image_detail
        .as_deref()
        .is_some_and(|detail| !["low", "high", "original", "auto"].contains(&detail))
    {
        return Err(invalid(
            name,
            "imageDetail must be low, high, original or auto",
        ));
    }
    Ok(options)
}
