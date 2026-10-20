//! Ports of the upstream `@ai-sdk/mistral` `mistral-embedding-model-options.test.ts`.
//!
//! The upstream zod schema `mistralEmbeddingModelOptions` is checked here by
//! `parse_mistral_provider_options`, fed the same options under the `mistral`
//! provider-options namespace.

use super::*;
use serde_json::json;

/// Parse embedding options given as the upstream `mistral` options object.
fn parse(options: Value) -> Result<MistralEmbeddingProviderOptions, AiMuxError> {
    let provider_options: SharedProviderOptions =
        serde_json::from_value(json!({ crate::mistral::options::NAMESPACE: options }))
            .expect("namespace map");
    parse_mistral_provider_options(Some(&provider_options))
}

mod mistral_embedding_model_options {
    use super::*;

    /// upstream: @ai-sdk/mistral src/mistral-embedding-model-options.test.ts › mistralEmbeddingModelOptions › accepts supported embedding provider options
    #[test]
    fn accepts_supported_embedding_provider_options() {
        let options = json!({
            "metadata": { "source": "test" },
            "outputDimension": 1024,
            "outputDtype": "float"
        });

        assert!(parse(options).is_ok());
    }

    /// upstream: @ai-sdk/mistral src/mistral-embedding-model-options.test.ts › mistralEmbeddingModelOptions › accepts all supported output dtypes
    #[test]
    fn accepts_all_supported_output_dtypes() {
        for output_dtype in ["float", "int8", "uint8", "binary", "ubinary"] {
            assert!(
                parse(json!({ "outputDtype": output_dtype })).is_ok(),
                "outputDtype {output_dtype} should be accepted"
            );
        }
    }

    /// upstream: @ai-sdk/mistral src/mistral-embedding-model-options.test.ts › mistralEmbeddingModelOptions › rejects invalid output dtype values
    #[test]
    fn rejects_invalid_output_dtype_values() {
        assert!(parse(json!({ "outputDtype": "base64" })).is_err());
    }
}
