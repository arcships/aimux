//! The provider registry: models addressed as `"{provider}:{model}"`.
//!
//! Mirrors the AI SDK's `createProviderRegistry`: the caller assembles a map
//! of provider ids to providers and the registry resolves a combined id by
//! splitting it at the first separator. The registry knows no provider on its
//! own and never falls back to another one.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::decision_model::DecisionModel;
use crate::embedding_model::EmbeddingModel;
use crate::error::AiMuxError;
use crate::files_model::Files;
use crate::image_model::ImageModel;
use crate::image_model_middleware::{ImageModelMiddleware, wrap_image_model};
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
    /// Middleware applied to each image model in input order.
    pub image_model_middleware: Vec<Arc<dyn ImageModelMiddleware>>,
}

impl Default for ProviderRegistryOptions {
    fn default() -> Self {
        Self {
            separator: ":".to_string(),
            language_model_middleware: Vec::new(),
            image_model_middleware: Vec::new(),
        }
    }
}

/// A set of providers addressed by id; see [`create_provider_registry`].
pub struct ProviderRegistry {
    providers: std::sync::RwLock<BTreeMap<String, Arc<dyn Provider>>>,
    separator: String,
    language_model_middleware: Vec<Arc<dyn LanguageModelMiddleware>>,
    image_model_middleware: Vec<Arc<dyn ImageModelMiddleware>>,
}

/// Create a registry over `providers` (the AI SDK's `createProviderRegistry`).
#[must_use]
pub fn create_provider_registry(
    providers: BTreeMap<String, Arc<dyn Provider>>,
    options: ProviderRegistryOptions,
) -> ProviderRegistry {
    ProviderRegistry {
        providers: std::sync::RwLock::new(providers),
        separator: options.separator,
        language_model_middleware: options.language_model_middleware,
        image_model_middleware: options.image_model_middleware,
    }
}

impl ProviderRegistry {
    /// The provider registered as `id`.
    ///
    /// # Errors
    ///
    /// [`AiMuxError::NoSuchProvider`] for an id that is not registered.
    pub fn provider(&self, id: &str) -> Result<Arc<dyn Provider>, AiMuxError> {
        self.get_provider(id, "")
    }

    fn get_provider(&self, id: &str, model_type: &str) -> Result<Arc<dyn Provider>, AiMuxError> {
        let providers = self
            .providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        providers
            .get(id)
            .cloned()
            .ok_or_else(|| AiMuxError::NoSuchProvider {
                provider_id: id.to_string(),
                model_id: id.to_string(),
                model_type: model_type.to_string(),
                available_providers: providers.keys().cloned().collect(),
            })
    }

    /// The registered provider ids, in order.
    pub fn provider_ids(&self) -> Vec<String> {
        let providers = self
            .providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        providers.keys().cloned().collect()
    }

    /// Whether a provider with `id` is currently registered.
    #[must_use]
    pub fn contains_provider(&self, id: &str) -> bool {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(id)
    }

    /// Number of providers currently registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Whether the registry currently holds no providers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Separator between provider id and model id when resolving compound ids.
    #[must_use]
    pub fn separator(&self) -> &str {
        &self.separator
    }

    /// Insert or replace `id` in place, returning the previous provider if any.
    pub fn register(
        &self,
        id: impl Into<String>,
        provider: Arc<dyn Provider>,
    ) -> Option<Arc<dyn Provider>> {
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.into(), provider)
    }

    /// Remove `id` in place, returning it when present.
    pub fn unregister(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
    }

    /// Insert or replace multiple providers in place.
    pub fn extend_providers(&self, entries: impl IntoIterator<Item = (String, Arc<dyn Provider>)>) {
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(entries);
    }

    /// Atomically replace the entire provider map in place.
    pub fn replace_providers(&self, providers: BTreeMap<String, Arc<dyn Provider>>) {
        let mut guard = self
            .providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = providers;
    }

    fn derive_with(&self, mutate: impl FnOnce(&mut BTreeMap<String, Arc<dyn Provider>>)) -> Self {
        let mut next = self
            .providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        mutate(&mut next);
        Self {
            providers: std::sync::RwLock::new(next),
            separator: self.separator.clone(),
            language_model_middleware: self.language_model_middleware.clone(),
            image_model_middleware: self.image_model_middleware.clone(),
        }
    }

    /// Return a new registry with `id` added or replaced without mutating `self`.
    #[must_use]
    pub fn with_provider(&self, id: impl Into<String>, provider: Arc<dyn Provider>) -> Self {
        self.derive_with(|next| {
            next.insert(id.into(), provider);
        })
    }

    /// Return a new registry without `id` without mutating `self`.
    #[must_use]
    pub fn without_provider(&self, id: &str) -> Self {
        self.derive_with(|next| {
            next.remove(id);
        })
    }

    /// Return a new registry with all `entries` added or replaced without mutating `self`.
    #[must_use]
    pub fn with_providers(
        &self,
        entries: impl IntoIterator<Item = (String, Arc<dyn Provider>)>,
    ) -> Self {
        self.derive_with(|next| {
            next.extend(entries);
        })
    }

    /// Return a new registry with the provider map replaced wholesale without mutating `self`.
    #[must_use]
    pub fn with_replaced_providers(&self, providers: BTreeMap<String, Arc<dyn Provider>>) -> Self {
        self.derive_with(|next| {
            *next = providers;
        })
    }

    fn offered_language_model(
        &self,
        id: &str,
        model_type: &str,
        select: impl FnOnce(
            &(dyn Provider + 'static),
            &str,
        ) -> Option<Result<Arc<dyn LanguageModel>, AiMuxError>>,
    ) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, model_type)?;
        let model = offered(select(provider.as_ref(), model_id), id, model_type)?;
        Ok(wrap_language_model(
            model,
            &self.language_model_middleware,
            None,
            None,
        ))
    }

    /// Resolve a language model via the provider's `.chat(id)` constructor.
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchProvider`] or [`AiMuxError::NoSuchModel`] when
    /// the provider or `.chat` method is unavailable, or whatever the provider
    /// returns.
    pub fn chat(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        self.offered_language_model(id, "chat", Provider::chat)
    }

    /// Resolve a language model via the provider's `.responses(id)` constructor.
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchProvider`] or [`AiMuxError::NoSuchModel`] when
    /// the provider or `.responses` method is unavailable, or whatever the provider
    /// returns.
    pub fn responses(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        self.offered_language_model(id, "responses", Provider::responses)
    }

    /// Resolve a language model via the provider's `.completion(id)` constructor.
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchProvider`] or [`AiMuxError::NoSuchModel`] when
    /// the provider or `.completion` method is unavailable, or whatever the provider
    /// returns.
    pub fn completion(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        self.offered_language_model(id, "completion", Provider::completion)
    }

    /// Resolve a language model via the provider's `.messages(id)` constructor.
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchProvider`] or [`AiMuxError::NoSuchModel`] when
    /// the provider or `.messages` method is unavailable, or whatever the provider
    /// returns.
    pub fn messages(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        self.offered_language_model(id, "messages", Provider::messages)
    }

    /// Split `id` at the first separator and look the provider up.
    fn resolve<'a>(
        &self,
        id: &'a str,
        model_type: &str,
    ) -> Result<(Arc<dyn Provider>, &'a str), AiMuxError> {
        let (provider_id, model_id) = id
            .split_once(self.separator.as_str())
            .ok_or_else(|| AiMuxError::no_such_model(id, model_type))?;
        Ok((self.get_provider(provider_id, model_type)?, model_id))
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
            None,
            None,
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
        Ok(wrap_image_model(
            provider.image_model(model_id)?,
            &self.image_model_middleware,
            None,
            None,
        ))
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

    /// The experimental decision model for a combined id
    /// (`ai@7.0.128 src/registry/provider-registry.ts:447-461`). A provider
    /// that offers none is [`AiMuxError::NoSuchModel`] with model type
    /// `"decisionModel"`.
    ///
    /// # Errors
    ///
    /// See [`Self::language_model`].
    pub fn decision_model(&self, id: &str) -> Result<Arc<dyn DecisionModel>, AiMuxError> {
        let (provider, model_id) = self.resolve(id, "decisionModel")?;
        offered(provider.decision_model(model_id), id, "decisionModel")
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
