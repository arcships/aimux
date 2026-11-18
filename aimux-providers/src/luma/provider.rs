//! The Luma provider (`luma-provider.ts`).

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::image_model::LumaImageModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.lumalabs.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "LUMA_API_KEY";
const DEFAULT_NAME: &str = "luma";

define_vendor_provider! {
    settings: LumaProviderSettings,
    provider: LumaProvider,
    create: create_luma,
    singleton: luma,
    display: "Luma",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: Some(crate::shared::aisdk_pins::LUMA),
}

impl LumaProvider {
    /// An image model (e.g. `"photon-1"`); `provider()` is `"{name}.image"`.
    #[must_use]
    pub fn image(&self, model_id: &str) -> LumaImageModel {
        LumaImageModel::from_config(model_id.to_string(), self.model_config("image"))
    }
}

crate::impl_single_modality_provider!(LumaProvider, image_model, |p, id| p.image(id));
