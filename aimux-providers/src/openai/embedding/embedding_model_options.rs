//! Provider options of the OpenAI embedding model (openai-embedding-model-options.ts).

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde_json::Value;

#[derive(Default, serde::Deserialize)]
pub(crate) struct OpenAIEmbeddingProviderOptions {
    pub(crate) dimensions: Option<serde_json::Number>,
    pub(crate) user: Option<String>,
}

pub(crate) fn parse_openai_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<OpenAIEmbeddingProviderOptions, AiMuxError> {
    // `.optional()` fields: an explicit `null` is as invalid as a wrong type.
    if let Some(openai) = options.and_then(|options| options.get("openai"))
        && (openai.get("dimensions").is_some_and(Value::is_null)
            || openai.get("user").is_some_and(Value::is_null))
    {
        return Err(AiMuxError::invalid_argument(
            "invalid openai provider options",
        ));
    }
    aimux_provider_utils::parse_provider_options("openai", options).map(Option::unwrap_or_default)
}
