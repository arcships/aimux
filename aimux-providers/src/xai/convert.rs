//! Helpers shared by the xAI Responses converter.
//!
//! Mirrors `supports-reasoning-effort.ts` and the provider-reference helper
//! used by upstream input conversion.

use std::collections::HashMap;

use aimux_core::{
    error::AiMuxError,
    language_model_message::FilePart,
    shared::{FileBytes, FileData},
};
use aimux_provider_utils::{
    MediaTypeData, detect_media_type, get_top_level_media_type, is_full_media_type,
};

// ── Reasoning effort ─────────────────────────────────────────────────────────

/// Models that reject the `reasoning_effort` parameter.
/// Matches `^grok-4\.20(-\d{4})?-(non-)?reasoning$`.
fn is_model_without_reasoning_effort(model_id: &str) -> bool {
    let rest = match model_id.strip_prefix("grok-4.20") {
        Some(r) => r,
        None => return false,
    };
    let rest = if rest.len() >= 5
        && rest.starts_with('-')
        && rest[1..5].chars().all(|c| c.is_ascii_digit())
    {
        &rest[5..]
    } else {
        rest
    };
    rest == "-reasoning" || rest == "-non-reasoning"
}

/// Whether the model accepts the `reasoning_effort` parameter.
#[must_use]
pub fn supports_reasoning_effort(model_id: &str) -> bool {
    !is_model_without_reasoning_effort(model_id)
}

/// Resolve the provider-specific reference string from a file part's
/// `provider` map.
///
/// # Errors
///
/// Returns a `String` listing the available providers when the requested key
/// is absent.
pub fn resolve_provider_reference(
    reference: &HashMap<String, String>,
    provider: &str,
) -> Result<String, String> {
    if let Some(value) = reference.get(provider) {
        return Ok(value.clone());
    }
    let mut available: Vec<&str> = reference.keys().map(String::as_str).collect();
    available.sort_unstable();
    Err(format!(
        "No provider reference found for provider '{}'. Available providers: {}",
        provider,
        available.join(", ")
    ))
}

pub(super) fn resolve_full_media_type(part: &FilePart) -> Result<String, AiMuxError> {
    let media_type = &part.media_type;
    if is_full_media_type(media_type) {
        return Ok(media_type.clone());
    }
    let data = match &part.data {
        FileData::Data {
            data: FileBytes::Binary(bytes),
        } => Some(MediaTypeData::Bytes(bytes)),
        FileData::Data {
            data: FileBytes::Base64(data),
        } => Some(MediaTypeData::Base64(data)),
        _ => None,
    };
    let reason = if let Some(data) = data {
        if let Some(detected) = detect_media_type(data, Some(get_top_level_media_type(media_type)))?
        {
            return Ok(detected.into());
        }
        "it could not be auto-detected"
    } else {
        "it is not passed as inline bytes"
    };
    Err(AiMuxError::UnsupportedFunctionality(format!(
        "file of media type \"{media_type}\" must specify subtype since {reason}"
    )))
}
