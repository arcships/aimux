//! Provider options of the OpenAI embedding model (openai-embedding-model-options.ts).

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

#[derive(Default, serde::Deserialize)]
pub(crate) struct OpenAIEmbeddingProviderOptions {
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) dimensions: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "aimux_provider_utils::optional_field")]
    pub(crate) user: Option<String>,
}

pub(crate) fn parse_openai_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<OpenAIEmbeddingProviderOptions, AiMuxError> {
    aimux_provider_utils::parse_provider_options("openai", options).map(Option::unwrap_or_default)
}
