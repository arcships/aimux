//! `cohere-embedding-model-options.ts`.

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

pub(super) struct CohereEmbeddingProviderOptions {
    pub(super) input_type: Option<String>,
    pub(super) truncate: Option<String>,
    pub(super) output_dimension: Option<u32>,
}

pub(super) fn parse_cohere_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<CohereEmbeddingProviderOptions, AiMuxError> {
    let provider_opts = super::chat_language_model_options::cohere_options(options);
    let string_option = |key: &str, allowed: &[&str]| -> Result<Option<String>, AiMuxError> {
        provider_opts
            .and_then(|o| o.get(key))
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| allowed.contains(value))
                    .map(str::to_owned)
                    .ok_or_else(|| AiMuxError::InvalidArgument(format!("Invalid cohere.{key}")))
            })
            .transpose()
    };
    let output_dimension = provider_opts
        .and_then(|o| o.get("outputDimension"))
        .map(|value| {
            value
                .as_f64()
                .filter(|value| [256.0, 512.0, 1024.0, 1536.0].contains(value))
                .map(|value| value as u32)
                .ok_or_else(|| AiMuxError::InvalidArgument("Invalid cohere.outputDimension".into()))
        })
        .transpose()?;
    Ok(CohereEmbeddingProviderOptions {
        input_type: string_option(
            "inputType",
            &[
                "search_document",
                "search_query",
                "classification",
                "clustering",
            ],
        )?,
        truncate: string_option("truncate", &["NONE", "START", "END"])?,
        output_dimension,
    })
}
