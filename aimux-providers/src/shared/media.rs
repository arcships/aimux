//! Shared media-input conversion helpers across audio and video models.

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;
use aimux_core::transcription_model::AudioInput;
use aimux_core::video_model::VideoFile;

/// Decode an [`AudioInput`] (raw bytes or base64 string) into raw bytes.
///
/// # Errors
///
/// Returns [`AiMuxError::InvalidArgument`] for URL inputs (which Core downloads
/// before calling `do_generate`) or invalid base64 payloads.
pub(crate) fn audio_input_to_bytes(audio: &AudioInput) -> Result<Vec<u8>, AiMuxError> {
    match audio {
        AudioInput::Binary(bytes) => Ok(bytes.clone()),
        AudioInput::Base64(b64) => aimux_provider_utils::convert_base64_to_uint8_array(b64)
            .map_err(|e| AiMuxError::InvalidArgument {
                message: format!("invalid base64: {e}"),
                cause: Some(ErrorCause::source(&e)),
            }),
        AudioInput::Url(_) => Err(AudioInput::url_error()),
    }
}

/// Convert an [`AudioInput`] (raw bytes or base64 string) into a base64 string.
///
/// # Errors
///
/// Returns [`AiMuxError::InvalidArgument`] for URL inputs (which Core downloads
/// before calling `do_generate`).
pub(crate) fn audio_input_to_base64(audio: &AudioInput) -> Result<String, AiMuxError> {
    match audio {
        AudioInput::Base64(value) => Ok(value.clone()),
        AudioInput::Binary(bytes) => Ok(aimux_provider_utils::convert_uint8_array_to_base64(bytes)),
        AudioInput::Url(_) => Err(AudioInput::url_error()),
    }
}

/// Convert a [`VideoFile`] into a URL string or raw base64 string for inline payloads.
pub(crate) fn video_file_to_url_or_base64(file: &VideoFile) -> String {
    match file {
        VideoFile::Url { url, .. } => url.clone(),
        VideoFile::File { data, .. } => aimux_provider_utils::convert_to_base64(data).into_owned(),
    }
}
