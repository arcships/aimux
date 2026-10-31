//! Port of `src/huggingface-provider.test.ts`.
//!
//! Harness differences from the TypeScript (expectations are unchanged):
//! - `typeof provider === 'function'` and the `[Function]` snapshot are checked
//!   by calling [`HuggingFaceProvider::call`], the Rust form of the callable
//!   provider.
//! - `typeof provider.responses === 'function'` is checked by a typed binding:
//!   the method exists and returns the Responses model.

use std::collections::HashMap;
use std::sync::Arc;

use crate::huggingface::{
    HuggingFaceProviderSettings, HuggingFaceResponsesModel, create_huggingface,
};
use aimux_core::language_model::LanguageModel;
use aimux_core::provider::Provider;

mod hugging_face_provider {
    use super::*;

    mod create_hugging_face {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/huggingface-provider.test.ts › HuggingFaceProvider › createHuggingFace › should create provider with default configuration
        #[test]
        fn should_create_provider_with_default_configuration() {
            let provider =
                create_huggingface(HuggingFaceProviderSettings::default()).expect("valid settings");

            let _model: Arc<dyn LanguageModel> = provider.call("any-model");
            let _responses: HuggingFaceResponsesModel = provider.responses("any-model");
            provider
                .language_model("any-model")
                .expect("language model");
        }

        /// upstream: @ai-sdk/huggingface src/huggingface-provider.test.ts › HuggingFaceProvider › createHuggingFace › should create provider with custom settings
        #[test]
        fn should_create_provider_with_custom_settings() {
            let provider = create_huggingface(HuggingFaceProviderSettings {
                api_key: Some("custom-key".to_string().into()),
                base_url: Some("https://custom.url".to_string()),
                headers: Some(HashMap::from([(
                    "Custom-Header".to_string(),
                    Some("test".to_string()),
                )])),
                ..Default::default()
            })
            .expect("valid settings");

            let _model: Arc<dyn LanguageModel> = provider.call("any-model");
            let _responses: HuggingFaceResponsesModel = provider.responses("any-model");
            provider
                .language_model("any-model")
                .expect("language model");
        }
    }

    mod model_creation_methods {
        use super::*;

        /// upstream: @ai-sdk/huggingface src/huggingface-provider.test.ts › HuggingFaceProvider › model creation methods › should expose responses method
        #[test]
        fn should_expose_responses_method() {
            let provider =
                create_huggingface(HuggingFaceProviderSettings::default()).expect("valid settings");

            let _responses: HuggingFaceResponsesModel = provider.responses("any-model");
        }

        /// upstream: @ai-sdk/huggingface src/huggingface-provider.test.ts › HuggingFaceProvider › model creation methods › should expose languageModel method
        #[test]
        fn should_expose_language_model_method() {
            let provider =
                create_huggingface(HuggingFaceProviderSettings::default()).expect("valid settings");

            let _model: Arc<dyn LanguageModel> = provider
                .language_model("any-model")
                .expect("language model");
        }
    }
}
