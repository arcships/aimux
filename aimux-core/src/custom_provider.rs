//! `custom_provider`: a provider assembled from alias tables, with an optional
//! fallback provider.
//!
//! Mirrors the AI SDK's `customProvider` (`ai/src/registry/custom-provider.ts`).
//! A lookup consults the alias table of its modality first, then the fallback
//! provider; with neither, it is [`AiMuxError::NoSuchModel`]. There are no
//! string aliases that resolve through a global default provider: Rust has
//! none.

use std::collections::HashMap;
use std::sync::Arc;

use crate::embedding_model::EmbeddingModel;
use crate::error::AiMuxError;
use crate::evaluation_model::EvaluationModel;
use crate::files_model::Files;
use crate::image_model::ImageModel;
use crate::language_model::LanguageModel;
use crate::provider::Provider;
use crate::reranking_model::RerankingModel;
use crate::search_model::SearchModel;
use crate::skills_model::Skills;
use crate::speech_model::SpeechModel;
use crate::transcription_model::TranscriptionModel;
use crate::video_model::VideoModel;

/// Options of [`custom_provider`]; every field is optional.
#[derive(Default)]
pub struct CustomProviderOptions {
    pub language_models: HashMap<String, Arc<dyn LanguageModel>>,
    pub embedding_models: HashMap<String, Arc<dyn EmbeddingModel>>,
    pub image_models: HashMap<String, Arc<dyn ImageModel>>,
    pub transcription_models: HashMap<String, Arc<dyn TranscriptionModel>>,
    pub speech_models: HashMap<String, Arc<dyn SpeechModel>>,
    pub reranking_models: HashMap<String, Arc<dyn RerankingModel>>,
    pub video_models: HashMap<String, Arc<dyn VideoModel>>,
    pub search_models: HashMap<String, Arc<dyn SearchModel>>,
    pub evaluation_models: HashMap<String, Arc<dyn EvaluationModel>>,
    /// A files interface for uploading files.
    pub files: Option<Arc<dyn Files>>,
    /// A skills interface for uploading skills.
    pub skills: Option<Arc<dyn Skills>>,
    /// Consulted when a requested model is not in the alias tables.
    pub fallback_provider: Option<Arc<dyn Provider>>,
}

/// A provider built by [`custom_provider`].
pub struct CustomProvider {
    options: CustomProviderOptions,
}

/// Create a custom provider from alias tables and an optional fallback.
#[must_use]
pub fn custom_provider(options: CustomProviderOptions) -> CustomProvider {
    CustomProvider { options }
}

/// The aliased model, else what the fallback offers, else `NoSuchModel`.
fn resolve<T: ?Sized>(
    models: &HashMap<String, Arc<T>>,
    id: &str,
    model_type: &str,
    fallback: impl FnOnce() -> Option<Result<Arc<T>, AiMuxError>>,
) -> Result<Arc<T>, AiMuxError> {
    models
        .get(id)
        .map(|model| Ok(Arc::clone(model)))
        .or_else(fallback)
        .unwrap_or_else(|| Err(AiMuxError::no_such_model(id, model_type)))
}

impl Provider for CustomProvider {
    fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        let o = &self.options;
        resolve(&o.language_models, id, "languageModel", || {
            o.fallback_provider.as_ref().map(|p| p.language_model(id))
        })
    }

    fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        let o = &self.options;
        resolve(&o.embedding_models, id, "embeddingModel", || {
            o.fallback_provider.as_ref().map(|p| p.embedding_model(id))
        })
    }

    fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        let o = &self.options;
        resolve(&o.image_models, id, "imageModel", || {
            o.fallback_provider.as_ref().map(|p| p.image_model(id))
        })
    }

    // The optional modalities are always defined here, as upstream: an id
    // neither table nor fallback serves is `NoSuchModel`, never `None`.

    fn transcription_model(
        &self,
        id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        let o = &self.options;
        Some(resolve(
            &o.transcription_models,
            id,
            "transcriptionModel",
            || o.fallback_provider.as_ref()?.transcription_model(id),
        ))
    }

    fn speech_model(&self, id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        let o = &self.options;
        Some(resolve(&o.speech_models, id, "speechModel", || {
            o.fallback_provider.as_ref()?.speech_model(id)
        }))
    }

    fn reranking_model(&self, id: &str) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        let o = &self.options;
        Some(resolve(&o.reranking_models, id, "rerankingModel", || {
            o.fallback_provider.as_ref()?.reranking_model(id)
        }))
    }

    fn video_model(&self, id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        let o = &self.options;
        Some(resolve(&o.video_models, id, "videoModel", || {
            o.fallback_provider.as_ref()?.video_model(id)
        }))
    }

    fn search_model(&self, id: &str) -> Option<Result<Arc<dyn SearchModel>, AiMuxError>> {
        let o = &self.options;
        Some(resolve(&o.search_models, id, "searchModel", || {
            o.fallback_provider.as_ref()?.search_model(id)
        }))
    }

    fn evaluation_model(&self, id: &str) -> Option<Result<Arc<dyn EvaluationModel>, AiMuxError>> {
        let o = &self.options;
        Some(resolve(&o.evaluation_models, id, "evaluationModel", || {
            o.fallback_provider.as_ref()?.evaluation_model(id)
        }))
    }

    fn files(&self) -> Option<Arc<dyn Files>> {
        let o = &self.options;
        o.files
            .clone()
            .or_else(|| o.fallback_provider.as_ref()?.files())
    }

    fn skills(&self) -> Option<Arc<dyn Skills>> {
        let o = &self.options;
        o.skills
            .clone()
            .or_else(|| o.fallback_provider.as_ref()?.skills())
    }
}
