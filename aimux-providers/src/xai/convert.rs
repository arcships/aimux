//! Helpers shared by the xAI Responses converter.
//!
//! Mirrors the TS xai package's `supports-reasoning-effort.ts` and
//! `remove-additional-properties.ts`, plus the media-type and provider
//! reference helpers its input conversion uses.

use serde_json::Value;

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

// ── JSON schema ──────────────────────────────────────────────────────────────

pub fn remove_additional_properties_false(value: &Value) -> Value {
    match value {
        Value::Array(arr) => {
            Value::Array(arr.iter().map(remove_additional_properties_false).collect())
        }
        Value::Object(obj) => {
            let mut result = serde_json::Map::new();
            for (key, val) in obj {
                if key == "additionalProperties" && val == &Value::Bool(false) {
                    continue;
                }
                result.insert(key.clone(), remove_additional_properties_false(val));
            }
            Value::Object(result)
        }
        other => other.clone(),
    }
}

// ── Media type helpers ───────────────────────────────────────────────────────

fn get_top_level_media_type(media_type: &str) -> &str {
    media_type.split('/').next().unwrap_or("")
}

#[must_use]
pub fn resolve_full_media_type(media_type: &str, b64_data: &str) -> String {
    let top_level = get_top_level_media_type(media_type);
    if top_level == "image" && media_type != "image" && !media_type.ends_with("/*") {
        return media_type.to_string();
    }
    if top_level == "image" {
        if b64_data.starts_with("iVBORw0KGgo") {
            return "image/png".to_string();
        }
        if b64_data.starts_with("/9j/") {
            return "image/jpeg".to_string();
        }
        if b64_data.starts_with("R0lGOD") {
            return "image/gif".to_string();
        }
        if b64_data.starts_with("UklGR") {
            return "image/webp".to_string();
        }
        return "image/png".to_string();
    }
    media_type.to_string()
}

/// Resolve the provider-specific reference string from a file part's
/// `provider` map.
///
/// # Errors
///
/// Returns a `String` listing the available providers when the requested key
/// is absent.
pub fn resolve_provider_reference(reference: &Value, provider: &str) -> Result<String, String> {
    if let Some(val) = reference.get(provider) {
        if let Some(s) = val.as_str() {
            return Ok(s.to_string());
        }
        return Ok(val.to_string());
    }
    let available: Vec<String> = reference
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Err(format!(
        "No provider reference found for provider '{}'. Available providers: {}",
        provider,
        available.join(", ")
    ))
}
