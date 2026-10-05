//! The provider registry: models addressed as `"{provider}:{model}"`.
//!
//! Mirrors the AI SDK's `createProviderRegistry`: the caller assembles a map
//! of provider ids to providers and the registry resolves a combined id by
//! splitting it at the first separator. The registry knows no provider on its
//! own and never falls back to another one.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::embedding_model::EmbeddingModel;
use crate::error::AiMuxError;
use crate::files_model::Files;
use crate::image_model::ImageModel;
use crate::language_model::LanguageModel;
use crate::language_model_middleware::{LanguageModelMiddleware, wrap_language_model};
use crate::provider::Provider;
use crate::reranking_model::RerankingModel;
use crate::search_model::SearchModel;
use crate::skills_model::Skills;
use crate::speech_model::SpeechModel;
use crate::transcription_model::TranscriptionModel;
use crate::video_model::VideoModel;

/// Options of [`create_provider_registry`].
#[derive(Clone)]
pub struct ProviderRegistryOptions {
    /// Separates the provider id from the model id. Defaults to `":"`.
    pub separator: String,
    /// Middleware applied to each language model in input order.
    pub language_model_middleware: Vec<Arc<dyn LanguageModelMiddleware>>,
}

impl Default for ProviderRegistryOptions {
    fn default() -> Self {
        Self {
            separator: ":".to_string(),
            language_model_middleware: Vec::new(),
        }
    }
}

/// A set of providers addressed by id; see [`create_provider_registry`].
pub struct ProviderRegistry {
    providers: BTreeMap<String, Arc<dyn Provider>>,
    separator: String,
    language_model_middleware: Vec<Arc<dyn LanguageModelMiddleware>>,
}

/// Create a registry over `providers` (the AI SDK's `createProviderRegistry`).
#[must_use]
pub fn create_provider_registry(
    providers: BTreeMap<String, Arc<dyn Provider>>,
    options: ProviderRegistryOptions,
) -> ProviderRegistry {
    ProviderRegistry {
        providers,
        separator: options.separator,
        language_model_middleware: options.language_model_middleware,
    }
}

impl ProviderRegistry {
    /// The provider registered as `id`.
    ///
    /// # Errors
    ///
    /// [`AiMuxError::NoSuchProvider`] for an id that is not registered.
    pub fn provider(&self, id: &str) -> Result<&Arc<dyn Provider>, AiMuxError> {
        self.providers
            .get(id)
            .ok_or_else(|| AiMuxError::NoSuchProvider {
                provider_id: id.to_string(),
            })
    }

    /// The registered provider ids, in order.
    pub fn provider_ids(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    /// Split `id` at the first separator and look the provider up.
    fn resolve<'a>(
        &self,
        id: &'a str,
        model_type: &str,
    ) -> Result<(&Arc<dyn Provider>, &'a str), AiMuxError> {
        let (provider_id, model_id) = id
            .split_once(self.separator.as_str())
            .ok_or_else(|| AiMuxError::no_such_model(id, model_type))?;
        Ok((self.provider(provider_id)?, model_id))
    }

    /// The language model `"{provider}{separator}{model}"`.
    ///
    /// # Errors
    ///
    /// [`AiMuxError::NoSuchModel`] for an id without the separator,
    /// [`AiMuxError::NoSuchProvider`] for an unregistered provider, and
    /// whatever the provider returns for the model id.
    pub fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "languageModel")?;
        Ok(wrap_language_model(
            provider.language_model(model_id)?,
            &self.language_model_middleware,
        ))
    }

    /// The embedding model for a combined id; errors as [`Self::language_model`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "embeddingModel")?;
        provider.embedding_model(model_id)
    }

    /// The image model for a combined id; errors as [`Self::language_model`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "imageModel")?;
        provider.image_model(model_id)
    }

    /// The transcription model for a combined id. A provider that offers none
    /// is [`AiMuxError::NoSuchModel`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn transcription_model(&self, id: &str) -> Result<Arc<dyn TranscriptionModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "transcriptionModel")?;
        offered(
            provider.transcription_model(model_id),
            id,
            "transcriptionModel",
        )
    }

    /// The speech model for a combined id. A provider that offers none is
    /// [`AiMuxError::NoSuchModel`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn speech_model(&self, id: &str) -> Result<Arc<dyn SpeechModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "speechModel")?;
        offered(provider.speech_model(model_id), id, "speechModel")
    }

    /// The reranking model for a combined id. A provider that offers none is
    /// [`AiMuxError::NoSuchModel`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn reranking_model(&self, id: &str) -> Result<Arc<dyn RerankingModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "rerankingModel")?;
        offered(provider.reranking_model(model_id), id, "rerankingModel")
    }

    /// The video model for a combined id. A provider that offers none is
    /// [`AiMuxError::NoSuchModel`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn video_model(&self, id: &str) -> Result<Arc<dyn VideoModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "videoModel")?;
        offered(provider.video_model(model_id), id, "videoModel")
    }

    /// The search model for a combined id (an aimux extension). A provider
    /// that offers none is [`AiMuxError::NoSuchModel`].
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn search_model(&self, id: &str) -> Result<Arc<dyn SearchModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "searchModel")?;
        offered(provider.search_model(model_id), id, "searchModel")
    }

    /// The skills interface of a registered provider.
    ///
    /// # Errors
    /// Returns an unknown provider or unsupported functionality error.
    pub fn skills(&self, provider_id: &str) -> Result<Arc<dyn Skills>, AiMuxError> {
        self.provider(provider_id)?.skills().ok_or_else(|| {
            AiMuxError::UnsupportedFunctionality(format!(
                "provider '{provider_id}' does not expose skills"
            ))
        })
    }

    /// The files interface of the provider `provider_id`.
    ///
    /// # Errors
    ///
    /// [`AiMuxError::NoSuchProvider`] for an unregistered provider,
    /// [`AiMuxError::UnsupportedFunctionality`] when it exposes no files.
    pub fn files(&self, provider_id: &str) -> Result<Arc<dyn Files>, AiMuxError> {
        self.provider(provider_id)?.files().ok_or_else(|| {
            AiMuxError::UnsupportedFunctionality(format!(
                "provider '{provider_id}' does not expose files"
            ))
        })
    }
}

fn offered<T: ?Sized>(
    model: Option<Result<Arc<T>, AiMuxError>>,
    id: &str,
    model_type: &str,
) -> Result<Arc<T>, AiMuxError> {
    model.unwrap_or_else(|| Err(AiMuxError::no_such_model(id, model_type)))
}
