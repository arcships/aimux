//! The providerOptions and providerMetadata namespace of xAI.
//!
//! `@ai-sdk/xai` reads `providerOptions.xai` and writes response metadata
//! under `xai`, whatever the provider is named. This module is the one place
//! that spells the key.

use aimux_core::error::AiMuxError;
use aimux_core::shared::{JsonObject, SharedProviderOptions, provider_namespace};
use aimux_core::types::ProviderMetadata;
use serde_json::Value;

/// The providerOptions / providerMetadata key (and the provider-reference key
/// of uploaded files).
pub(crate) const NAMESPACE: &str = "xai";

/// The xAI options in a providerOptions container (a call's map, a part's
/// object).
pub(crate) fn xai_options(provider_options: Option<&SharedProviderOptions>) -> Option<&JsonObject> {
    provider_options?.get(NAMESPACE)
}

/// `{ "xai": payload }`.
pub(crate) fn xai_metadata(payload: Value) -> ProviderMetadata {
    provider_namespace(NAMESPACE, payload).expect("metadata payload is an object")
}

// ── Validation of the media-family options (the AI SDK parses them with zod
// and throws `InvalidArgumentError` on a violation; `null` counts as unset
// where the schema says `.nullish()`). ──

fn invalid(key: &str, expected: &str) -> AiMuxError {
    AiMuxError::InvalidArgument(format!(
        "Invalid argument for parameter providerOptions: xai.{key} must be {expected}"
    ))
}

fn present<'a>(options: &'a JsonObject, key: &str) -> Option<&'a Value> {
    options.get(key).filter(|value| !value.is_null())
}

pub(crate) fn opt_bool(options: &JsonObject, key: &str) -> Result<Option<bool>, AiMuxError> {
    present(options, key).map_or(Ok(None), |v| {
        v.as_bool()
            .map(Some)
            .ok_or_else(|| invalid(key, "a boolean"))
    })
}

pub(crate) fn opt_string(options: &JsonObject, key: &str) -> Result<Option<String>, AiMuxError> {
    present(options, key).map_or(Ok(None), |v| {
        v.as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| invalid(key, "a string"))
    })
}

pub(crate) fn opt_non_empty_string(
    options: &JsonObject,
    key: &str,
) -> Result<Option<String>, AiMuxError> {
    match opt_string(options, key)? {
        Some(s) if s.is_empty() => Err(invalid(key, "a non-empty string")),
        other => Ok(other),
    }
}

/// A string from a closed set (`z.enum`).
pub(crate) fn opt_enum(
    options: &JsonObject,
    key: &str,
    allowed: &[&str],
) -> Result<Option<String>, AiMuxError> {
    match opt_string(options, key)? {
        Some(s) if !allowed.contains(&s.as_str()) => {
            Err(invalid(key, &format!("one of {}", allowed.join(", "))))
        }
        other => Ok(other),
    }
}

/// An integer within `min..=max` (`z.number().int().min().max()`), or one of
/// `only` when it is non-empty (a `z.union` of literals).
pub(crate) fn opt_int(
    options: &JsonObject,
    key: &str,
    min: i64,
    max: i64,
    only: &[i64],
) -> Result<Option<i64>, AiMuxError> {
    present(options, key).map_or(Ok(None), |v| {
        let n = v
            .as_i64()
            .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64));
        match n {
            Some(n) if (min..=max).contains(&n) && (only.is_empty() || only.contains(&n)) => {
                Ok(Some(n))
            }
            _ => Err(invalid(key, "an allowed integer")),
        }
    })
}

/// A number within `min..=max`.
pub(crate) fn opt_f64(
    options: &JsonObject,
    key: &str,
    min: f64,
    max: f64,
) -> Result<Option<f64>, AiMuxError> {
    present(options, key).map_or(Ok(None), |v| match v.as_f64() {
        Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
        _ => Err(invalid(key, "a number in range")),
    })
}

/// `keyterm`: a string or an array of strings.
pub(crate) fn keyterms(options: &JsonObject) -> Result<Vec<String>, AiMuxError> {
    match present(options, "keyterm") {
        None => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("keyterm", "a string or an array of strings"))
            })
            .collect(),
        Some(_) => Err(invalid("keyterm", "a string or an array of strings")),
    }
}

pub(crate) fn sample_rate(options: &JsonObject) -> Result<Option<i64>, AiMuxError> {
    opt_int(
        options,
        "sampleRate",
        0,
        i64::MAX,
        &[8000, 16000, 22050, 24000, 44100, 48000],
    )
}
