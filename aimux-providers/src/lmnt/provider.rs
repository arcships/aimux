//! LMNT provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::speech_model::LMNTSpeechModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.lmnt.com";
pub(crate) const API_KEY_ENV_VAR: &str = "LMNT_API_KEY";
const DEFAULT_NAME: &str = "lmnt";

define_vendor_provider! {
    settings: LMNTProviderSettings,
    provider: LMNTProvider,
    create: create_lmnt,
    singleton: lmnt,
    display: "LMNT",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("x-api-key"),
    extra_headers: Vec::new(),
    package: None,
}

impl LMNTProvider {
    /// A speech (TTS) model (e.g. `"aurora"`); `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self, model_id: &str) -> LMNTSpeechModel {
        LMNTSpeechModel::from_config(model_id.to_string(), self.model_config("speech"))
    }
}

crate::impl_single_modality_provider!(LMNTProvider, speech_model, |p, id| p.speech(id));
