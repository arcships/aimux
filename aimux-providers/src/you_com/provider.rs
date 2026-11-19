use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::YouComSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://ydc-index.io";
pub(crate) const API_KEY_ENV_VAR: &str = "YDC_API_KEY";
const DEFAULT_NAME: &str = "you_com";

define_vendor_provider! {
    settings: YouComProviderSettings,
    provider: YouComProvider,
    create: create_you_com,
    singleton: you_com,
    display: "You.com",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("X-API-Key"),
    extra_headers: Vec::new(),
    package: None,
}

impl YouComProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> YouComSearchModel {
        YouComSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(YouComProvider, search_model, |p, _id| p.search_model());
