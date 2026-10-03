//! The `Provider` trait — a factory that creates model instances for every
//! modality a vendor offers.
//!
//! Mirrors the AI SDK's `ProviderV4`: three required model constructors
//! (language / embedding / image), four optional ones that default to "not
//! offered", and a `files()` accessor. Runtime model discovery lives on the
//! separate [`ProviderDiscovery`] trait.

use std::sync::Arc;

use futures::future::BoxFuture;

use crate::embedding_model::EmbeddingModel;
use crate::error::AiMuxError;
use crate::files_model::Files;
use crate::image_model::ImageModel;
use crate::language_model::LanguageModel;
use crate::model_catalogue::RuntimeModel;
use crate::reranking_model::RerankingModel;
use crate::search_model::SearchModel;
use crate::speech_model::SpeechModel;
use crate::transcription_model::TranscriptionModel;
use crate::video_model::VideoModel;

/// A provider factory.
///
/// Holds API keys / config and creates model instances by model id. The
/// shape follows AI SDK `ProviderV4`:
///
/// - [`language_model`](Self::language_model),
///   [`embedding_model`](Self::embedding_model) and
///   [`image_model`](Self::image_model) are required. A vendor that does not
///   offer a modality returns [`AiMuxError::NoSuchModel`] carrying the
///   matching `model_type` (`"languageModel"`, `"embeddingModel"`,
///   `"imageModel"`). Credential or settings errors are never reported as
///   `NoSuchModel`.
/// - [`transcription_model`](Self::transcription_model),
///   [`speech_model`](Self::speech_model),
///   [`reranking_model`](Self::reranking_model) and [`files`](Self::files)
///   are optional: `None` means the vendor does not offer that modality.
/// - [`video_model`](Self::video_model) and
///   [`search_model`](Self::search_model) are aimux extensions; they are not
///   members of `ProviderV4`.
///
/// A provider has no `name()`: the name it was registered under belongs to
/// whoever holds the registry, and each model reports its own
/// [`LanguageModel::provider`] string.
pub trait Provider: Send + Sync {
    /// Create a language model by its id (e.g. `"gpt-4o"`).
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchModel`] with `model_type: "languageModel"`
    /// when the vendor offers no language models; implementations may also
    /// return lookup/auth errors for ids they cannot serve.
    fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError>;

    /// Create an embedding model by its id (e.g. `"text-embedding-3-large"`).
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchModel`] with `model_type: "embeddingModel"`
    /// when the vendor offers no embedding models.
    fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError>;

    /// Create an image model by its id (e.g. `"dall-e-3"`).
    ///
    /// # Errors
    ///
    /// Returns [`AiMuxError::NoSuchModel`] with `model_type: "imageModel"`
    /// when the vendor offers no image models.
    fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError>;

    /// Create a transcription (STT) model; `None` when not offered.
    fn transcription_model(
        &self,
        id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        let _ = id;
        None
    }

    /// Create a speech (TTS) model; `None` when not offered.
    fn speech_model(&self, id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        let _ = id;
        None
    }

    /// Create a reranking model; `None` when not offered.
    fn reranking_model(&self, id: &str) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        let _ = id;
        None
    }

    /// The vendor's file-management interface; `None` when not offered.
    fn files(&self) -> Option<Arc<dyn Files>> {
        None
    }

    /// Create a video-generation model; `None` when not offered.
    ///
    /// aimux extension — not a member of AI SDK `ProviderV4`.
    fn video_model(&self, id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        let _ = id;
        None
    }

    /// Create a web-search model; `None` when not offered.
    ///
    /// aimux extension — not a member of AI SDK `ProviderV4`.
    fn search_model(&self, id: &str) -> Option<Result<Arc<dyn SearchModel>, AiMuxError>> {
        let _ = id;
        None
    }
}

/// Runtime model discovery, implemented by providers that expose a
/// model-list endpoint (RFC-0027).
///
/// Separate from [`Provider`] because `ProviderV4` has no equivalent and most
/// vendors cannot list models.
pub trait ProviderDiscovery: Send + Sync {
    /// List the models this account can call on this provider, via the
    /// provider's `/models` endpoint.
    ///
    /// Returns **only the provider's official data** (`RuntimeModel`: id,
    /// owned_by, created) — no community catalogue enrichment. To supplement
    /// with model specs (context length, capabilities, reasoning portrait),
    /// call `get_model_specs` separately and merge in the host (RFC-0027).
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<RuntimeModel>, AiMuxError>>;
}
