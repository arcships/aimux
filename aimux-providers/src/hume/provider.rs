//! Hume provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::speech_model::HumeSpeechModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.hume.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "HUME_API_KEY";
const DEFAULT_NAME: &str = "hume";

define_vendor_provider! {
    settings: HumeProviderSettings,
    provider: HumeProvider,
    create: create_hume,
    singleton: hume,
    display: "Hume",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("X-Hume-Api-Key"),
    extra_headers: Vec::new(),
    package: None,
}

impl HumeProvider {
    /// The speech (TTS) model. Hume does not use model ids for TTS, so the model id is always an empty string; `provider()` is `"{name}.speech"`.
    #[must_use]
    pub fn speech(&self) -> HumeSpeechModel {
        HumeSpeechModel::from_config(self.model_config("speech"))
    }
}

crate::impl_single_modality_provider!(HumeProvider, speech_model, |p, _id| p.speech());
