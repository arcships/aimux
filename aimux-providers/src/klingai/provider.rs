//! KlingAI provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::video_model::KlingAIVideoModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.klingai.com";
pub(crate) const API_KEY_ENV_VAR: &str = "KLINGAI_API_KEY";
const DEFAULT_NAME: &str = "klingai";

define_vendor_provider! {
    settings: KlingAIProviderSettings,
    provider: KlingAIProvider,
    create: create_klingai,
    singleton: klingai,
    display: "KlingAI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl KlingAIProvider {
    /// A video model (e.g. `"kling-v2.6-t2v"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> KlingAIVideoModel {
        KlingAIVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

crate::impl_single_modality_provider!(KlingAIProvider, video_model, |p, id| p.video(id));
