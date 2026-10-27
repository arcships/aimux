//! `google-error.ts`: Gemini's error body and the failed-response handler.

use serde::Deserialize;
use serde_json::Value;

use aimux_core::error::AiMuxError;

// ── Provider error response ──────────────────────────────────────────────────

/// Google error envelope: `{ "error": { "code": 400, "message": "...", "status": "INVALID_ARGUMENT" } }`.
#[derive(Debug, Deserialize)]
pub struct GoogleErrorEnvelope {
    pub error: GoogleError,
}

#[derive(Debug, Deserialize)]
pub struct GoogleError {
    #[serde(default)]
    pub code: Option<i64>,
    pub message: String,
    #[serde(default)]
    pub status: Option<String>,
}

pub(crate) fn google_stream_error(
    error: &GoogleError,
    url: &str,
    request_body_values: Value,
    response_headers: std::collections::HashMap<String, String>,
) -> AiMuxError {
    let status_code = error
        .code
        .and_then(|status| u16::try_from(status).ok())
        .filter(|status| (400..=599).contains(status));
    let data = serde_json::json!({
        "error": {
            "code": error.code,
            "message": error.message,
            "status": error.status,
        }
    });
    aimux_provider_utils::stream_error_api_call(
        error.message.clone(),
        error.status.clone(),
        status_code,
        &data,
        url,
        request_body_values,
        response_headers,
    )
}

pub(crate) fn google_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let error = data.get("error").unwrap_or(data);
        aimux_provider_utils::ProviderErrorParts {
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            provider_code: error
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    })
}
