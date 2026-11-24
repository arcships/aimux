//! Compose a text preset with a native image/video provider without replacing
//! the preset's existing model factories or discovery implementation.

use std::sync::Arc;

use aimux_core::provider::ProviderDiscovery;

use aimux_core::{
    AiMuxError, DecisionModel, EmbeddingModel, Files, ImageModel, LanguageModel, Provider,
    RerankingModel, SearchModel, Skills, SpeechModel, TranscriptionModel, VideoModel,
};

/// Which image and video model IDs the overlay answers from `media`.
enum NativeIds {
    /// Every ID, whatever the media provider does with it.
    All,
    /// Only the listed IDs. A model that exists only on a relay keeps the
    /// preset's implementation.
    Only {
        image: &'static [&'static str],
        video: &'static [&'static str],
    },
}

impl NativeIds {
    fn serves_image(&self, id: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only { image, .. } => image.contains(&id),
        }
    }

    fn serves_video(&self, id: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only { video, .. } => video.contains(&id),
        }
    }
}

pub(crate) struct MediaOverlay {
    base: Arc<dyn Provider>,
    media: Arc<dyn Provider>,
    native_ids: NativeIds,
}

impl MediaOverlay {
    /// Every image and video ID is served by `media`.
    pub(crate) fn new(base: Arc<dyn Provider>, media: Arc<dyn Provider>) -> Self {
        Self {
            base,
            media,
            native_ids: NativeIds::All,
        }
    }

    /// Only `image_models` and `video_models` are served by `media`; any other
    /// ID is looked up on `base`.
    pub(crate) fn with_native_ids(
        base: Arc<dyn Provider>,
        media: Arc<dyn Provider>,
        image_models: &'static [&'static str],
        video_models: &'static [&'static str],
    ) -> Self {
        Self {
            base,
            media,
            native_ids: NativeIds::Only {
                image: image_models,
                video: video_models,
            },
        }
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
        if self.native_ids.serves_image(id) {
            self.media.image_model(id)
        } else {
            self.base.image_model(id)
        }
    }

    fn video_model(&self, id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        if self.native_ids.serves_video(id) {
            self.media.video_model(id)
        } else {
            self.base.video_model(id)
        }
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
