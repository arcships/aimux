//! RunwayML provider factory and configuration.

use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::video_model::RunwaymlVideoModel;

const RUNWAY_VERSION: &str = "2024-11-06";

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.dev.runwayml.com";
pub(crate) const API_KEY_ENV_VAR: &str = "RUNWAYML_API_SECRET";
const DEFAULT_NAME: &str = "runwayml";

define_vendor_provider! {
    settings: RunwaymlProviderSettings,
    provider: RunwaymlProvider,
    create: create_runwayml,
    singleton: runwayml,
    display: "RunwayML",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: vec![("X-Runway-Version".to_string(), RUNWAY_VERSION.to_string())],
    package: None,
}

impl RunwaymlProvider {
    /// A video model (e.g. `"gen4_turbo"`); `provider()` is `"{name}.video"`.
    #[must_use]
    pub fn video(&self, model_id: &str) -> RunwaymlVideoModel {
        RunwaymlVideoModel::from_config(model_id.to_string(), self.model_config("video"))
    }
}

crate::impl_single_modality_provider!(RunwaymlProvider, video_model, |p, id| p.video(id));
