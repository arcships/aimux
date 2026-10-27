//! `mistral-embedding-model-options.ts`.

use serde_json::Value;

use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;

pub(super) struct MistralEmbeddingProviderOptions {
    pub(super) metadata: Option<Value>,
    pub(super) output_dimension: Option<Value>,
    pub(super) output_dtype: Option<String>,
}

pub(super) fn parse_mistral_provider_options(
    options: Option<&SharedProviderOptions>,
) -> Result<MistralEmbeddingProviderOptions, AiMuxError> {
    let provider_opts = super::chat_language_model_options::mistral_options(options);
    let metadata = provider_opts.and_then(|o| o.get("metadata")).cloned();
    let output_dimension = provider_opts
        .and_then(|o| o.get("outputDimension"))
        .cloned();
    let output_dtype = provider_opts.and_then(|o| o.get("outputDtype"));
    if metadata.as_ref().is_some_and(|value| !value.is_object()) {
        return Err(AiMuxError::invalid_argument(
            "mistral.metadata must be a record",
        ));
    }
    if output_dimension.as_ref().is_some_and(|value| {
        !value.as_f64().is_some_and(|number| {
            number > 0.0 && number.fract() == 0.0 && number <= 9_007_199_254_740_991.0
        })
    }) {
        return Err(AiMuxError::invalid_argument(
            "mistral.outputDimension must be a positive integer",
        ));
    }
    if output_dtype.is_some_and(|value| {
        !matches!(
            value.as_str(),
            Some("float" | "int8" | "uint8" | "binary" | "ubinary")
        )
    }) {
        return Err(AiMuxError::invalid_argument(
            "mistral.outputDtype must be float, int8, uint8, binary, or ubinary",
        ));
    }
    Ok(MistralEmbeddingProviderOptions {
        metadata,
        output_dimension,
        output_dtype: output_dtype.and_then(Value::as_str).map(str::to_owned),
    })
}
