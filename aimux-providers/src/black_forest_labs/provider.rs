//! Black Forest Labs provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::image_model::BlackForestLabsImageModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.bfl.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "BFL_API_KEY";
const DEFAULT_NAME: &str = "blackForestLabs";

define_vendor_provider! {
    settings: BlackForestLabsProviderSettings,
    provider: BlackForestLabsProvider,
    create: create_black_forest_labs,
    singleton: black_forest_labs,
    display: "Black Forest Labs",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl BlackForestLabsProvider {
    /// An image model (e.g. `"flux-pro-1.1"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> BlackForestLabsImageModel {
        BlackForestLabsImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(BlackForestLabsProvider, image_model, |p, id| p.image(id));
