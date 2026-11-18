//! AssemblyAI provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::transcription_model::AssemblyAITranscriptionModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.assemblyai.com";
pub(crate) const API_KEY_ENV_VAR: &str = "ASSEMBLYAI_API_KEY";
const DEFAULT_NAME: &str = "assemblyai";

define_vendor_provider! {
    settings: AssemblyAIProviderSettings,
    provider: AssemblyAIProvider,
    create: create_assemblyai,
    singleton: assemblyai,
    display: "AssemblyAI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("Authorization"),
    extra_headers: Vec::new(),
    package: None,
}

impl AssemblyAIProvider {
    /// A transcription (STT) model (e.g. `"universal-3-5-pro"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> AssemblyAITranscriptionModel {
        AssemblyAITranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(AssemblyAIProvider, transcription_model, |p, id| p
    .transcription(id));
