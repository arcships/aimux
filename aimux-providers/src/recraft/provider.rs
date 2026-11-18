//! Recraft provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::image_model::RecraftImageModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://external.api.recraft.ai/v1";
pub(crate) const API_KEY_ENV_VAR: &str = "RECRAFT_API_TOKEN";
const DEFAULT_NAME: &str = "recraft";

define_vendor_provider! {
    settings: RecraftProviderSettings,
    provider: RecraftProvider,
    create: create_recraft,
    singleton: recraft,
    display: "Recraft",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl RecraftProvider {
    /// An image model (e.g. `"recraftv3"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> RecraftImageModel {
        RecraftImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(RecraftProvider, image_model, |p, id| p.image(id));
