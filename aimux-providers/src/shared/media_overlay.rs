//! Compose a text preset with a native image/video provider without replacing
//! the preset's existing model factories or discovery implementation.

use std::sync::Arc;

use aimux_core::provider::ProviderDiscovery;
use aimux_core::{
    AiMuxError, DecisionModel, EmbeddingModel, Files, ImageModel, LanguageModel, Provider,
    RerankingModel, SearchModel, Skills, SpeechModel, TranscriptionModel, VideoModel,
};

pub(crate) struct MediaOverlay {
    base: Arc<dyn Provider>,
    media: Arc<dyn Provider>,
}

impl MediaOverlay {
    pub(crate) fn new(base: Arc<dyn Provider>, media: Arc<dyn Provider>) -> Self {
        Self { base, media }
    }
}

impl Provider for MediaOverlay {
    fn specification_version(&self) -> &'static str {
        self.base.specification_version()
    }

    fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        self.base.language_model(id)
    }

    fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        self.base.embedding_model(id)
    }

    fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        self.media.image_model(id)
    }

    fn video_model(&self, id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        self.media.video_model(id)
    }

    fn transcription_model(
        &self,
        id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        self.base.transcription_model(id)
    }

    fn speech_model(&self, id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        self.base.speech_model(id)
    }

    fn reranking_model(&self, id: &str) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        self.base.reranking_model(id)
    }

    fn decision_model(&self, id: &str) -> Option<Result<Arc<dyn DecisionModel>, AiMuxError>> {
        self.base.decision_model(id)
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        self.base.files()
    }

    fn skills(&self) -> Option<Arc<dyn Skills>> {
        self.base.skills()
    }

    fn search_model(&self, id: &str) -> Option<Result<Arc<dyn SearchModel>, AiMuxError>> {
        self.base.search_model(id)
    }

    fn discovery(&self) -> Option<&dyn ProviderDiscovery> {
        self.base.discovery()
    }
}
