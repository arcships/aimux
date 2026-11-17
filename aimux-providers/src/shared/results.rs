//! Pieces of a modality result that the vendor packages without an upstream
//! package build the same way: an unsupported-setting warning, the error of a
//! job the vendor reports as failed, and the response tail.

use serde_json::Value;

use aimux_core::Timestamp;
use aimux_core::error::{AiMuxError, ApiCallError, ApiCallErrorOrigin};
use aimux_core::image_model::ImageResponse;
use aimux_core::shared::{SharedHeaders, Warning};
use aimux_core::speech_model::SpeechResponse;
use aimux_core::transcription_model::TranscriptionResponse;
use aimux_core::video_model::VideoResponse;

/// `Warning::Unsupported` for `feature`, with `details`.
pub(crate) fn unsupported(feature: &str, details: impl Into<String>) -> Warning {
    Warning::Unsupported {
        feature: feature.to_string(),
        details: Some(details.into()),
    }
}

/// The error of an asynchronous job that the vendor answered with HTTP 200
/// but a failed status: `provider_code` is that status, `url` the request
/// that reported it and `response_body` its body.
pub(crate) fn task_failed(
    message: impl Into<String>,
    url: impl Into<String>,
    provider_code: impl Into<String>,
    response_body: Option<String>,
) -> AiMuxError {
    AiMuxError::ApiCall(Box::new(ApiCallError {
        status_code: Some(200),
        provider_code: Some(provider_code.into()),
        response_body,
        origin: ApiCallErrorOrigin::HttpResponse,
        ..ApiCallError::new(message, url, serde_json::json!({}))
    }))
}

/// The response tail of an image result.
pub(crate) fn image_response(
    timestamp: Timestamp,
    model_id: &str,
    headers: SharedHeaders,
) -> ImageResponse {
    ImageResponse {
        timestamp: Some(timestamp),
        model_id: Some(model_id.to_string()),
        headers: Some(headers),
    }
}

/// The response tail of a video result.
pub(crate) fn video_response(
    timestamp: Timestamp,
    model_id: &str,
    headers: SharedHeaders,
) -> VideoResponse {
    VideoResponse {
        timestamp: Some(timestamp),
        model_id: Some(model_id.to_string()),
        headers: Some(headers),
    }
}

/// The response tail of a speech result; the audio is the result, so there
/// is no body.
pub(crate) fn speech_response(
    timestamp: Timestamp,
    model_id: &str,
    headers: SharedHeaders,
) -> SpeechResponse {
    SpeechResponse {
        timestamp: Some(timestamp),
        model_id: Some(model_id.to_string()),
        headers: Some(headers),
        body: None,
    }
}

/// The response tail of a transcription result, with the raw response body.
pub(crate) fn transcription_response(
    timestamp: Timestamp,
    model_id: &str,
    headers: SharedHeaders,
    body: Value,
) -> TranscriptionResponse {
    TranscriptionResponse {
        timestamp: Some(timestamp),
        model_id: Some(model_id.to_string()),
        headers: Some(headers),
        body: Some(body),
    }
}
