//! Gladia provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::transcription_model::GladiaTranscriptionModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.gladia.io";
pub(crate) const API_KEY_ENV_VAR: &str = "GLADIA_API_KEY";
const DEFAULT_NAME: &str = "gladia";

define_vendor_provider! {
    settings: GladiaProviderSettings,
    provider: GladiaProvider,
    create: create_gladia,
    singleton: gladia,
    display: "Gladia",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl GladiaProvider {
    /// A transcription (STT) model; `provider()` is `"{name}.transcription"`.
    #[must_use]
    pub fn transcription(&self, model_id: &str) -> GladiaTranscriptionModel {
        GladiaTranscriptionModel::from_config(
            model_id.to_string(),
            self.model_config("transcription"),
        )
    }
}

crate::impl_single_modality_provider!(GladiaProvider, transcription_model, |p, id| p
    .transcription(id));
