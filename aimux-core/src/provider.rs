//! The `Provider` trait — a factory that creates `LanguageModel` instances.

use std::future::Future;
use std::pin::Pin;

use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::model_catalogue::ResolvedModel;

/// A provider factory.
///
/// Holds API keys / config and creates `LanguageModel` instances by model name.
pub trait Provider: Send + Sync {
    /// Unique provider name (e.g. `"openai"`).
    fn name(&self) -> &str;

    /// Create a model instance by its name string (e.g. `"gpt-4o"`).
<<<<<<< HEAD
    ///
    /// Non-language-model providers (image/video/speech/search/… — e.g. Tavily,
    /// Stability, Recraft) do not implement this and get the default
    /// `Unsupported` error. Only providers that actually expose a language
    /// model override it (issue M9).
    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported(format!(
            "provider '{}' does not provide language models",
            self.name()
        )))
=======
    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError>;

    /// List the models this account can call on this provider (runtime
    /// discovery via the provider's `/models` endpoint), optionally enriched
    /// with a static capability portrait from community knowledge (anya2a).
    ///
    /// Default returns [`AiMuxError::Unsupported`] — providers that expose a
    /// model-list endpoint override this. The returned [`ResolvedModel`] list is
    /// **advisory**: callers read `spec` to decide how to configure requests;
    /// aimux never auto-applies it in the request path (RFC-0027).
    ///
    /// Implemented as a `Pin<Box<Future>>` (rather than `#[async_trait]`) so that
    /// the dozens of existing `Provider` impls need no changes — only providers
    /// that actually support `/models` override this.
    fn list_models(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ResolvedModel>, AiMuxError>> + Send + '_>> {
        let name = self.name().to_string();
        Box::pin(async move {
            Err(AiMuxError::Unsupported(format!(
                "list_models not implemented for provider '{name}'"
            )))
        })
>>>>>>> 63f0188 (feat(rfc0027): P1 — Provider::list_models + anya2a catalogue)
    }
}
