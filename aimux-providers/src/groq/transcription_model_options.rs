//! `groq-transcription-model-options.ts`.

use serde::Deserialize;
use serde_json::Value;

use aimux_core::ErrorCause;
use aimux_core::error::AiMuxError;

use super::chat_language_model_options::NAMESPACE;

/// `groqTranscriptionModelOptions` (`groq-transcription-model-options.ts`):
/// the `groq` namespace of a transcription call. `nullish` fields are
/// `Option`s, so `null` and absent both mean unset.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroqTranscriptionModelOptions {
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub response_format: Option<String>,
    /// 0 to 1 inclusive.
    pub temperature: Option<f64>,
    pub timestamp_granularities: Option<Vec<String>>,
}

/// Parse the `groq` namespace of a transcription call (`parseProviderOptions`).
///
/// # Errors
///
/// `InvalidArgument` when an option has the wrong type or `temperature` is
/// outside 0..=1.
pub(crate) fn parse_groq_transcription_options(
    provider_options: Option<&aimux_core::shared::SharedProviderOptions>,
) -> Result<Option<GroqTranscriptionModelOptions>, AiMuxError> {
    crate::openai::convert::parse_chat_provider_options(provider_options, NAMESPACE, |groq| {
        let parsed: GroqTranscriptionModelOptions =
            serde_json::from_value(Value::Object(groq.clone())).map_err(|error| {
                AiMuxError::InvalidArgument {
                    message: format!("invalid argument for parameter providerOptions: {error}"),
                    cause: Some(ErrorCause::source(&error)),
                }
            })?;
        if parsed
            .temperature
            .is_some_and(|t| !(0.0..=1.0).contains(&t))
        {
            return Err(AiMuxError::invalid_argument(
                "invalid argument for parameter providerOptions: temperature must be between 0 and 1"
                    .to_string(),
            ));
        }
        Ok(parsed)
    })
}
