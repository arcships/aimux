//! fal error bodies (`fal-error.ts`).

use serde_json::Value;

use aimux_core::error::AiMuxError;

/// fal errors are FastAPI-style: `{"detail": "..."}` or
/// `{"detail": [{"loc": [...], "msg": "...", "type": "..."}]}` where `type`
/// is the machine code (https://fal.ai/docs/model-apis/errors).
///
/// fal-image-model.ts reads a validation error as one `loc.join('.'): msg`
/// line per entry, and any other error as its `message` (fal-error.ts); the
/// two are handled the same way here, so a validation error keeps every
/// entry instead of the first.
pub(super) fn fal_error_parts(data: &Value) -> aimux_provider_utils::ProviderErrorParts {
    let detail = data.get("detail");
    let entries = detail.and_then(Value::as_array);
    let message = entries
        .filter(|entries| !entries.is_empty())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let loc = entry
                        .get("loc")
                        .and_then(Value::as_array)
                        .map(|loc| {
                            loc.iter()
                                .map(|part| match part {
                                    Value::String(part) => part.clone(),
                                    other => other.to_string(),
                                })
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default();
                    let msg = entry.get("msg").and_then(Value::as_str).unwrap_or_default();
                    format!("{loc}: {msg}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .or_else(|| detail.and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            data.get("error")
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            data.get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Unknown fal error".to_string());
    aimux_provider_utils::ProviderErrorParts {
        message,
        provider_code: entries
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.get("type"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// Whether a failed status poll only says the job has not finished: its body
/// is `{"detail": "Request is still in progress"}`, whatever the status code
/// (the status `failedResponseHandler` of fal-transcription-model.ts and
/// fal-video-model.ts).
pub(super) fn is_in_progress(error: &AiMuxError) -> bool {
    let AiMuxError::ApiCall(detail) = error else {
        return false;
    };
    detail
        .response_body
        .as_deref()
        .and_then(|body| serde_json::from_str::<Value>(body).ok())
        .is_some_and(|body| body["detail"] == "Request is still in progress")
}

pub(super) fn fal_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError> {
    aimux_provider_utils::create_json_error_response_handler(fal_error_parts)
}
