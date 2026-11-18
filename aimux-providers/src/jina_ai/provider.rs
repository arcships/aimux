use aimux_provider_utils::AuthScheme;

use crate::shared::define_vendor_provider;

use super::reranking_model::JinaAiRerankingModel;

pub(crate) const DEFAULT_BASE_URL: &str = "https://api.jina.ai";
pub(crate) const API_KEY_ENV_VAR: &str = "JINA_AI_API_KEY";
const DEFAULT_NAME: &str = "jina_ai";

define_vendor_provider! {
    settings: JinaAiProviderSettings,
    provider: JinaAiProvider,
    create: create_jina_ai,
    singleton: jina_ai,
    display: "Jina AI",
    default_base_url: DEFAULT_BASE_URL,
    api_key_env_var: API_KEY_ENV_VAR,
    default_name: DEFAULT_NAME,
    auth: AuthScheme::Bearer,
    extra_headers: Vec::new(),
    package: None,
}

impl JinaAiProvider {
    /// A reranking model (e.g. `"jina-reranker-v2-base-multilingual"`); `provider()` is `"{name}.reranking"`.
    #[must_use]
    pub fn reranking_model(&self, model_id: &str) -> JinaAiRerankingModel {
        JinaAiRerankingModel::from_config(model_id.to_string(), self.model_config("reranking"))
    }
}

crate::impl_single_modality_provider!(JinaAiProvider, reranking_model, |p, id| p
    .reranking_model(id));
