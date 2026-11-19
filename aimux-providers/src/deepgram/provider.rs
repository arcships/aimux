//! The Deepgram provider (`deepgram-provider.ts`).

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::transcription_model::DeepgramTranscriptionModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.deepgram.com";
pub(crate) const API_KEY_ENV_VAR: &str = "DEEPGRAM_API_KEY";
const DEFAULT_NAME: &str = "deepgram";

define_vendor_provider! {
    settings: DeepgramProviderSettings,
    provider: DeepgramProvider,
    create: create_deepgram,
    singleton: deepgram,
    display: "Deepgram",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Scheme("Token"),
    extra_headers: Vec::new(),
    package: Some(crate::shared::aisdk_pins::DEEPGRAM),
}

impl DeepgramProvider {
    /// A transcription (STT) model (e.g. `"nova-3"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> DeepgramTranscriptionModel {
        DeepgramTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(DeepgramProvider, transcription_model, |p, id| p
    .transcription(id));
