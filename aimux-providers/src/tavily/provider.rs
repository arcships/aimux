use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::TavilySearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.tavily.com";
pub(crate) const API_KEY_ENV_VAR: &str = "TAVILY_API_KEY";
const DEFAULT_NAME: &str = "tavily";

define_vendor_provider! {
    settings: TavilyProviderSettings,
    provider: TavilyProvider,
    create: create_tavily,
    singleton: tavily,
    display: "Tavily",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl TavilyProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> TavilySearchModel {
        TavilySearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(TavilyProvider, search_model, |p, _id| p.search_model());
