//! Provider options of the OpenAI embedding model (openai-embedding-model-options.ts).

use aimux_core::ErrorCause;
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
    match options.and_then(|options| options.get("openai")) {
        Some(value) => {
            if value
                .get("dimensions")
                .is_some_and(|value| !value.is_number())
                || value.get("user").is_some_and(|value| !value.is_string())
            {
                return Err(AiMuxError::invalid_argument(
                    "invalid openai embedding options",
                ));
            }
            serde_json::from_value(Value::Object(value.clone())).map_err(|error| {
                AiMuxError::InvalidArgument {
                    message: format!("invalid openai embedding options: {error}"),
                    cause: Some(ErrorCause::source(&error)),
                }
            })
        }
        None => Ok(OpenAIEmbeddingProviderOptions::default()),
    }
}
