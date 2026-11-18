use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::TinyfishSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.search.tinyfish.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "TINYFISH_API_KEY";
const DEFAULT_NAME: &str = "tinyfish";

define_vendor_provider! {
    settings: TinyfishProviderSettings,
    provider: TinyfishProvider,
    create: create_tinyfish,
    singleton: tinyfish,
    display: "TinyFish",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("X-API-Key"),
    extra_headers: Vec::new(),
    package: None,
}

impl TinyfishProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> TinyfishSearchModel {
        TinyfishSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(TinyfishProvider, search_model, |p, _id| p.search_model());
