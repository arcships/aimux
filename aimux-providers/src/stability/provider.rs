//! Stability provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::image_model::StabilityImageModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.stability.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "STABILITY_API_KEY";
const DEFAULT_NAME: &str = "stability";

define_vendor_provider! {
    settings: StabilityProviderSettings,
    provider: StabilityProvider,
    create: create_stability,
    singleton: stability,
    display: "Stability",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: vec![("Accept".to_string(), "image/*".to_string())],
    package: None,
}

impl StabilityProvider {
    /// An image model (e.g. `"stable-image-core"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> StabilityImageModel {
        StabilityImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(StabilityProvider, image_model, |p, id| p.image(id));
