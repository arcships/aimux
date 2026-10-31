//! Ports of the upstream `@ai-sdk/mistral` `mistral-embedding-model-options.test.ts`.
//!
//! The upstream zod schema `mistralEmbeddingModelOptions` is checked here by
//! `parse_mistral_provider_options`, fed the same options under the `mistral`
//! provider-options namespace.

use crate::mistral::embedding_model_options::{
    MistralEmbeddingProviderOptions, parse_mistral_provider_options,
};
use aimux_core::error::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde_json::{Value, json};

/// Parse embedding options given as the upstream `mistral` options object.
fn parse(options: Value) -> Result<MistralEmbeddingProviderOptions, AiMuxError> {
    let provider_options: SharedProviderOptions = serde_json::from_value(
        json!({ crate::mistral::chat_language_model_options::NAMESPACE: options }),
    )
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

/// aimux's own: the refinements of the schema fail the same way as a wrong
/// type, with the upstream message.
mod refinements {
    use super::*;

    #[test]
    fn output_dimension_is_a_positive_integer() {
        assert!(parse(json!({ "outputDimension": 2.0 })).is_ok());
        for value in [json!(0), json!(-1), json!(1.5), json!("8"), Value::Null] {
            let error = parse(json!({ "outputDimension": value })).err().unwrap();
            let AiMuxError::InvalidArgument { message, cause } = &error else {
                panic!("expected InvalidArgument for {value}, got {error:?}");
            };
            assert_eq!(
                message,
                "Invalid argument for parameter providerOptions: invalid mistral provider options"
            );
            assert!(cause.is_some(), "{error:?}");
        }
    }

    #[test]
    fn metadata_is_a_record() {
        assert!(parse(json!({ "metadata": ["a"] })).is_err());
        assert!(parse(json!({ "metadata": {} })).is_ok());
    }
}
