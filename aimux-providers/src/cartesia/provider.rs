//! Cartesia provider factory and configuration.

use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::speech_model::CartesiaSpeechModel;
use super::transcription_model::CartesiaTranscriptionModel;

/// The Cartesia API version sent with every request via the `Cartesia-Version`
/// header.
const CARTESIA_API_VERSION: &str = "2026-03-01";

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "cartesia";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.cartesia.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "CARTESIA_API_KEY";
const DEFAULT_NAME: &str = "cartesia";

pub(super) fn cartesia_failed_response_handler() -> aimux_provider_utils::ResponseHandler<AiMuxError>
{
    aimux_provider_utils::create_json_error_response_handler(|data| {
        let title = data
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Cartesia error");
        let message = data
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        aimux_provider_utils::ProviderErrorParts {
            message: format!("{title}: {message}"),
            provider_code: data
                .get("error_code")
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    })
}

define_vendor_provider! {
    settings: CartesiaProviderSettings,
    provider: CartesiaProvider,
    create: create_cartesia,
    singleton: cartesia,
    display: "Cartesia",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: vec![(
        "Cartesia-Version".to_string(),
        CARTESIA_API_VERSION.to_string(),
    )],
    package: None,
}

impl CartesiaProvider {
    /// A speech (TTS) model (e.g. `"sonic-3.5"`); `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> CartesiaSpeechModel {
        CartesiaSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }

    /// A transcription (STT) model (e.g. `"ink-whisper"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> CartesiaTranscriptionModel {
        CartesiaTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_modality_provider!(CartesiaProvider {
    transcription_model: |p, id| p.transcription(id),
    speech_model: |p, id| p.speech(id),
});
