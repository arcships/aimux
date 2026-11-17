use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::search_model::ParallelAiSearchModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.parallel.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "PARALLEL_API_KEY";
const DEFAULT_NAME: &str = "parallel_ai";

define_vendor_provider! {
    settings: ParallelAiProviderSettings,
    provider: ParallelAiProvider,
    create: create_parallel_ai,
    singleton: parallel_ai,
    display: "Parallel AI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Header("x-api-key"),
    extra_headers: Vec::new(),
    package: None,
}

impl ParallelAiProvider {
    /// The search model; `provider()` is `"{name}.search"`.
    #[must_use]
    pub fn search_model(&self) -> ParallelAiSearchModel {
        ParallelAiSearchModel::from_config(self.model_config("search"))
    }
}

crate::impl_single_modality_provider!(ParallelAiProvider, search_model, |p, _id| p.search_model());
