//! Rev.ai provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::transcription_model::RevaiTranscriptionModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.rev.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "REVAI_API_KEY";
const DEFAULT_NAME: &str = "revai";

define_vendor_provider! {
    settings: RevaiProviderSettings,
    provider: RevaiProvider,
    create: create_revai,
    singleton: revai,
    display: "Rev.ai",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl RevaiProvider {
    /// A transcription (STT) model (e.g. `"machine"`); `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> RevaiTranscriptionModel {
        RevaiTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(RevaiProvider, transcription_model, |p, id| p
    .transcription(id));
