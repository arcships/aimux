//! Provider-level middleware: the AI SDK's `wrapProvider`
//! (ai@7.0.127 `middleware/wrap-provider.ts:20-53`).

use std::sync::Arc;

use crate::embedding_model::EmbeddingModel;
use crate::error::AiMuxError;
use crate::files_model::Files;
use crate::image_model::ImageModel;
use crate::image_model_middleware::{ImageModelMiddleware, wrap_image_model};
use crate::language_model::LanguageModel;
use crate::language_model_middleware::{LanguageModelMiddleware, wrap_language_model};
use crate::provider::{Provider, ProviderDiscovery};
use crate::reranking_model::RerankingModel;
use crate::search_model::SearchModel;
use crate::skills_model::Skills;
use crate::speech_model::SpeechModel;
use crate::transcription_model::TranscriptionModel;
use crate::video_model::VideoModel;

/// Wrap every language model of `provider` with `language_model_middleware`
/// and, when `image_model_middleware` is not empty, every image model with
/// it. The first middleware of each list is the outermost, as in
/// [`wrap_language_model`] and [`wrap_image_model`].
///
/// Every other member is the wrapped provider's own: embedding,
/// transcription, speech and reranking models, files and skills, as
/// upstream, and the aimux extensions (video, search and decision models,
/// discovery), which are members of [`Provider`] here.
#[must_use]
pub fn wrap_provider(
    provider: Arc<dyn Provider>,
    language_model_middleware: &[Arc<dyn LanguageModelMiddleware>],
    image_model_middleware: &[Arc<dyn ImageModelMiddleware>],
) -> Arc<dyn Provider> {
    Arc::new(WrappedProvider {
        provider,
        language_model_middleware: language_model_middleware.to_vec(),
        image_model_middleware: image_model_middleware.to_vec(),
    })
}

struct WrappedProvider {
    provider: Arc<dyn Provider>,
    language_model_middleware: Vec<Arc<dyn LanguageModelMiddleware>>,
    image_model_middleware: Vec<Arc<dyn ImageModelMiddleware>>,
}

impl Provider for WrappedProvider {
    fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        Ok(wrap_language_model(
            self.provider.language_model(id)?,
            &self.language_model_middleware,
            None,
            None,
        ))
    }

    fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        self.provider.embedding_model(id)
    }

    fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        Ok(wrap_image_model(
            self.provider.image_model(id)?,
            &self.image_model_middleware,
            None,
            None,
        ))
    }

    fn transcription_model(
        &self,
        id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        self.provider.transcription_model(id)
    }

    fn speech_model(&self, id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        self.provider.speech_model(id)
    }

    fn reranking_model(&self, id: &str) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        self.provider.reranking_model(id)
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        self.provider.files()
    }

    fn skills(&self) -> Option<Arc<dyn Skills>> {
        self.provider.skills()
    }

    fn decision_model(
        &self,
        id: &str,
    ) -> Option<Result<Arc<dyn crate::DecisionModel>, AiMuxError>> {
        self.provider.decision_model(id)
    }

    fn video_model(&self, id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        self.provider.video_model(id)
    }

    fn search_model(&self, id: &str) -> Option<Result<Arc<dyn SearchModel>, AiMuxError>> {
        self.provider.search_model(id)
    }

    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        self.provider.discovery()
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::embedding_model::{EmbeddingCallOptions, EmbeddingResult};
    use crate::image_model::{ImageCallOptions, ImageResult};
    use crate::options::CallOptions;
    use crate::result::{GenerateResult, StreamResult};

    struct Language;

    #[async_trait]
    impl LanguageModel for Language {
        fn provider(&self) -> &str {
            "mock.chat"
        }

        fn model_id(&self) -> &str {
            "language"
        }

        async fn do_generate(&self, _: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            unreachable!("not called")
        }

        async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> {
            unreachable!("not called")
        }
    }

    struct Image;

    #[async_trait]
    impl ImageModel for Image {
        fn provider(&self) -> &str {
            "mock.image"
        }

        fn model_id(&self) -> &str {
            "image"
        }

        fn max_images_per_call(&self) -> Option<u32> {
            None
        }

        async fn do_generate(&self, _: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
            unreachable!("not called")
        }
    }

    struct Embedding;

    #[async_trait]
    impl EmbeddingModel for Embedding {
        fn provider(&self) -> &str {
            "mock.embedding"
        }

        fn model_id(&self) -> &str {
            "embedding"
        }

        fn max_embeddings_per_call(&self) -> Option<u32> {
            None
        }

        fn supports_parallel_calls(&self) -> bool {
            true
        }

        async fn do_embed(&self, _: &EmbeddingCallOptions) -> Result<EmbeddingResult, AiMuxError> {
            unreachable!("not called")
        }
    }

    struct Mock {
        image: Arc<dyn ImageModel>,
        embedding: Arc<dyn EmbeddingModel>,
    }

    impl Provider for Mock {
        fn language_model(&self, _: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
            Ok(Arc::new(Language))
        }

        fn embedding_model(&self, _: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
            Ok(self.embedding.clone())
        }

        fn image_model(&self, _: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
            Ok(self.image.clone())
        }
    }

    struct RenameLanguage;

    impl LanguageModelMiddleware for RenameLanguage {
        fn override_model_id(&self, model: &dyn LanguageModel) -> Option<String> {
            Some(format!("wrapped-{}", model.model_id()))
        }
    }

    struct RenameImage;

    impl ImageModelMiddleware for RenameImage {
        fn override_model_id(&self, model: &dyn ImageModel) -> Option<String> {
            Some(format!("wrapped-{}", model.model_id()))
        }
    }

    fn mock() -> (
        Arc<dyn Provider>,
        Arc<dyn ImageModel>,
        Arc<dyn EmbeddingModel>,
    ) {
        let image: Arc<dyn ImageModel> = Arc::new(Image);
        let embedding: Arc<dyn EmbeddingModel> = Arc::new(Embedding);
        let provider = Arc::new(Mock {
            image: image.clone(),
            embedding: embedding.clone(),
        });
        (provider, image, embedding)
    }

    /// `wrapProvider` (ai@7.0.127 `wrap-provider.ts:31-52`): language models
    /// are always wrapped, image models only when image middleware is given,
    /// and every other member is the provider's own.
    #[test]
    fn language_models_are_wrapped_and_other_members_pass_through() {
        let (provider, image, embedding) = mock();
        let wrapped = wrap_provider(provider, &[Arc::new(RenameLanguage)], &[]);

        let language = wrapped.language_model("language").unwrap();
        assert_eq!(language.model_id(), "wrapped-language");
        assert_eq!(language.provider(), "mock.chat");
        assert!(Arc::ptr_eq(&wrapped.image_model("image").unwrap(), &image));
        assert!(Arc::ptr_eq(
            &wrapped.embedding_model("embedding").unwrap(),
            &embedding
        ));
        assert!(wrapped.transcription_model("t").is_none());
        assert!(wrapped.files().is_none());
        assert_eq!(wrapped.specification_version(), "v4");
    }

    #[test]
    fn image_models_are_wrapped_when_image_middleware_is_given() {
        let (provider, _, _) = mock();
        let wrapped = wrap_provider(
            provider,
            &[Arc::new(RenameLanguage)],
            &[Arc::new(RenameImage)],
        );
        assert_eq!(
            wrapped.image_model("image").unwrap().model_id(),
            "wrapped-image"
        );
        assert_eq!(
            wrapped.language_model("language").unwrap().model_id(),
            "wrapped-language"
        );
    }
}
