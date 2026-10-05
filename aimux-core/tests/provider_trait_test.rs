//! Shape tests for the `Provider` trait (AI SDK `ProviderV4`).
//!
//! A provider that only offers language models must answer the two other
//! required constructors with `NoSuchModel { model_type }`, and every optional
//! constructor with `None`.

use std::sync::Arc;

use async_trait::async_trait;

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::{EmbeddingModel, ImageModel, LanguageModel, Provider};

struct OnlyLanguageModel {
    model_id: String,
}

#[async_trait]
impl LanguageModel for OnlyLanguageModel {
    fn provider(&self) -> &str {
        "only-lm.chat"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn do_generate(&self, _options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        Err(AiMuxError::Other("not used in this test".into()))
    }

    async fn do_stream(&self, _options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        Err(AiMuxError::Other("not used in this test".into()))
    }
}

struct OnlyLanguageProvider;

impl Provider for OnlyLanguageProvider {
    fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(Arc::new(OnlyLanguageModel {
            model_id: id.to_string(),
        }))
    }

    fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(id, "embeddingModel"))
    }

    fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Err(AiMuxError::no_such_model(id, "imageModel"))
    }
}

#[test]
fn language_model_is_served() {
    let model = OnlyLanguageProvider.language_model("m-1").unwrap();
    assert_eq!(model.model_id(), "m-1");
    assert_eq!(model.provider(), "only-lm.chat");
}

#[test]
fn unsupported_embedding_model_is_no_such_model() {
    match OnlyLanguageProvider.embedding_model("e-1") {
        Err(AiMuxError::NoSuchModel {
            model_id,
            model_type,
        }) => {
            assert_eq!(model_id, "e-1");
            assert_eq!(model_type, "embeddingModel");
        }
        _ => panic!("expected NoSuchModel for embeddingModel"),
    }
}

#[test]
fn unsupported_image_model_is_no_such_model() {
    match OnlyLanguageProvider.image_model("i-1") {
        Err(AiMuxError::NoSuchModel {
            model_id,
            model_type,
        }) => {
            assert_eq!(model_id, "i-1");
            assert_eq!(model_type, "imageModel");
        }
        _ => panic!("expected NoSuchModel for imageModel"),
    }
}

#[test]
fn optional_constructors_default_to_none() {
    let p = OnlyLanguageProvider;
    assert!(p.speech_model("s").is_none());
    assert!(p.transcription_model("t").is_none());
    assert!(p.reranking_model("r").is_none());
    assert!(p.video_model("v").is_none());
    assert!(p.search_model("q").is_none());
    assert!(p.files().is_none());
}

#[test]
fn provider_is_object_safe() {
    let p: Arc<dyn Provider> = Arc::new(OnlyLanguageProvider);
    assert!(p.language_model("m").is_ok());
}

#[test]
fn supported_urls_default_is_empty() {
    let model = OnlyLanguageProvider.language_model("m").unwrap();
    let urls = model.supported_urls();
    assert!(urls.is_empty());
    assert!(urls.0.is_empty());
}

#[test]
fn supported_urls_can_be_overridden_per_media_type() {
    use aimux_core::language_model::SupportedUrls;

    struct Images;

    #[async_trait]
    impl LanguageModel for Images {
        fn provider(&self) -> &str {
            "images.chat"
        }
        fn model_id(&self) -> &str {
            "m"
        }
        async fn do_generate(&self, _o: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            Err(AiMuxError::Other("unused".into()))
        }
        async fn do_stream(&self, _o: &CallOptions) -> Result<StreamResult, AiMuxError> {
            Err(AiMuxError::Other("unused".into()))
        }
        fn supported_urls(&self) -> SupportedUrls {
            let mut map = std::collections::HashMap::new();
            map.insert(
                "image/*".to_string(),
                vec![regex::Regex::new(r"^https://cdn\.example\.com/").unwrap()],
            );
            SupportedUrls(map)
        }
    }

    let urls = Images.supported_urls();
    assert!(!urls.is_empty());
    assert!(urls.0["image/*"][0].is_match("https://cdn.example.com/a.png"));
    assert!(!urls.0["image/*"][0].is_match("https://other.example.com/a.png"));
}

// ── Provider registry ────────────────────────────────────────────────────────
// Ports of `packages/ai/src/registry/provider-registry.test.ts` (ai@7.0.127),
// the `languageModel` and `transcriptionModel` groups.

mod registry {
    use std::collections::BTreeMap;

    use aimux_core::{ProviderRegistryOptions, create_provider_registry};

    use super::*;

    fn providers() -> BTreeMap<String, Arc<dyn Provider>> {
        BTreeMap::from([(
            "provider".to_string(),
            Arc::new(OnlyLanguageProvider) as Arc<dyn Provider>,
        )])
    }

    /// TS: should return language model from provider
    #[test]
    fn returns_the_language_model_from_the_provider() {
        let registry = create_provider_registry(providers(), ProviderRegistryOptions::default());
        let model = registry.language_model("provider:model").unwrap();
        assert_eq!(model.model_id(), "model");
    }

    /// TS: should return language model with additional colon from provider
    #[test]
    fn splits_at_the_first_separator_only() {
        let registry = create_provider_registry(providers(), ProviderRegistryOptions::default());
        let model = registry.language_model("provider:model:part2").unwrap();
        assert_eq!(model.model_id(), "model:part2");
    }

    /// TS: should throw NoSuchProviderError if provider does not exist
    #[test]
    fn an_unknown_provider_is_no_such_provider() {
        let registry =
            create_provider_registry(BTreeMap::new(), ProviderRegistryOptions::default());
        let error = registry.language_model("provider:model").err().unwrap();
        assert!(
            matches!(error, AiMuxError::NoSuchProvider { ref provider_id } if provider_id == "provider")
        );
    }

    /// TS: should throw NoSuchModelError if model id doesn't contain a colon
    #[test]
    fn an_id_without_the_separator_is_no_such_model() {
        let registry = create_provider_registry(providers(), ProviderRegistryOptions::default());
        let error = registry.language_model("model").err().unwrap();
        assert!(
            matches!(error, AiMuxError::NoSuchModel { ref model_type, .. } if model_type == "languageModel")
        );
    }

    /// TS (transcriptionModel): should throw NoSuchModelError if provider
    /// does not return a model
    #[test]
    fn a_modality_the_provider_does_not_offer_is_no_such_model() {
        let registry = create_provider_registry(providers(), ProviderRegistryOptions::default());
        let error = registry
            .transcription_model("provider:model")
            .err()
            .unwrap();
        assert!(
            matches!(error, AiMuxError::NoSuchModel { ref model_type, .. } if model_type == "transcriptionModel")
        );
    }
}
