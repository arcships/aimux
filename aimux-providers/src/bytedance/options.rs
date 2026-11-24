//! Validated Ark wire options and shared error classification.

use aimux_core::{AiMuxError, shared::SharedProviderOptions};
use serde_json::{Map, Value};

pub(super) fn options(
    source: &SharedProviderOptions,
    allowed: &[&str],
) -> Result<Map<String, Value>, AiMuxError> {
    let values = source.get("bytedance").cloned().unwrap_or_default();
    for key in values.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(AiMuxError::invalid_argument(format!(
                "unsupported bytedance option `{key}`"
            )));
        }
    }
    Ok(values)
}

/// Clear the retryability of a billed request.
///
/// Used after a bounded inner exchange (such as an asset download) or for
/// errors that must never resubmit a generation Core already paid for.
pub(super) fn no_replay(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            detail.is_retryable = false;
            AiMuxError::ApiCall(detail)
        }
        other => other,
    }
}

/// Clear the retryability of a create, except for a rate limit.
///
/// A create the vendor refused for any other reason queued no task or image,
/// so replaying it could bill twice. A rate-limited create (HTTP 429) is safe
/// to replay and stays retryable.
pub(super) fn not_retryable_unless_rate_limited(error: AiMuxError) -> AiMuxError {
    match error {
        AiMuxError::ApiCall(mut detail) => {
            if detail.status_code != Some(429) {
                detail.is_retryable = false;
            }
            AiMuxError::ApiCall(detail)
        }
        other => other,
    }
}

pub(super) fn failed_response() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    crate::shared::nested_error_response_handler("Ark request failed")
}

pub(super) fn http_url(value: &str) -> Result<(), AiMuxError> {
    let parsed = url::Url::parse(value)
        .map_err(|_| AiMuxError::invalid_argument("expected an absolute HTTP(S) URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AiMuxError::invalid_argument(
            "expected an absolute HTTP(S) URL",
        ));
    }
    Ok(())
}

pub(super) fn image_url(value: &str) -> Result<(), AiMuxError> {
    if let Some(data) = value.strip_prefix("data:image/") {
        let (_, payload) = data
            .split_once(";base64,")
            .ok_or_else(|| AiMuxError::invalid_argument("expected a base64 image data URI"))?;
        let bytes = aimux_provider_utils::convert_base64_to_uint8_array(payload)
            .map_err(|_| AiMuxError::invalid_argument("invalid base64 image data"))?;
        if bytes.is_empty() {
            return Err(AiMuxError::invalid_argument("image data must not be empty"));
        }
        Ok(())
    } else {
        http_url(value)
    }
}

pub(super) fn check_types(
    values: &Map<String, Value>,
    strings: &[&str],
    integers: &[&str],
    booleans: &[&str],
    objects: &[&str],
) -> Result<(), AiMuxError> {
    for (key, value) in values {
        let valid = if strings.contains(&key.as_str()) {
            value.is_string()
        } else if integers.contains(&key.as_str()) {
            value.as_i64().is_some() || value.as_u64().is_some()
        } else if booleans.contains(&key.as_str()) {
            value.is_boolean()
        } else if objects.contains(&key.as_str()) {
            value.is_object()
        } else {
            true
        };
        if !valid {
            return Err(AiMuxError::invalid_argument(format!(
                "invalid type for bytedance `{key}`"
            )));
        }
    }
    Ok(())
}
