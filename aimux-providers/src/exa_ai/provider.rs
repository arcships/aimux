use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::ExaAiSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.exa.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "EXA_API_KEY";
const DEFAULT_NAME: &str = "exa_ai";

define_vendor_provider! {
    settings: ExaAiProviderSettings,
    provider: ExaAiProvider,
    create: create_exa_ai,
    singleton: exa_ai,
    display: "Exa AI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("x-api-key"),
    extra_headers: Vec::new(),
    package: None,
}

impl ExaAiProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> ExaAiSearchModel {
        ExaAiSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(ExaAiProvider, search_model, |p, _id| p.search_model());
