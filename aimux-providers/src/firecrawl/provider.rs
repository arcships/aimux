use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::FirecrawlSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.firecrawl.dev";
pub(crate) const API_KEY_ENV_VAR: &str = "FIRECRAWL_API_KEY";
const DEFAULT_NAME: &str = "firecrawl";

define_vendor_provider! {
    settings: FirecrawlProviderSettings,
    provider: FirecrawlProvider,
    create: create_firecrawl,
    singleton: firecrawl,
    display: "Firecrawl",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl FirecrawlProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> FirecrawlSearchModel {
        FirecrawlSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(FirecrawlProvider, search_model, |p, _id| p.search_model());
