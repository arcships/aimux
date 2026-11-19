use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::SerperSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://google.serper.dev";
pub(crate) const API_KEY_ENV_VAR: &str = "SERPER_API_KEY";
const DEFAULT_NAME: &str = "serper";

define_vendor_provider! {
    settings: SerperProviderSettings,
    provider: SerperProvider,
    create: create_serper,
    singleton: serper,
    display: "Serper",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("X-API-KEY"),
    extra_headers: Vec::new(),
    package: None,
}

impl SerperProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> SerperSearchModel {
        SerperSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(SerperProvider, search_model, |p, _id| p.search_model());
