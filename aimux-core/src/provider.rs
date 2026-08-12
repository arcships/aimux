//! The `Provider` trait — a factory that creates `LanguageModel` instances.

use std::future::Future;
use std::pin::Pin;

use crate::error::AiMuxError;
use crate::language_model::LanguageModel;
use crate::model_catalogue::RuntimeModel;

/// A provider factory.
///
/// Holds API keys / config and creates `LanguageModel` instances by model name.
pub trait Provider: Send + Sync {
    /// Unique provider name (e.g. `"openai"`).
    fn name(&self) -> &str;

    /// Create a model instance by its name string (e.g. `"gpt-4o"`).
    ///
    /// Non-language-model providers (image/video/speech/search/… — e.g. Tavily,
    /// Stability, Recraft) do not implement this and get the default
    /// `Unsupported` error. Only providers that actually expose a language
    /// model override it (issue M9).
    fn language_model(&self, _model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::UnsupportedFunctionality(format!(
            "provider '{}' does not provide language models",
            self.name()
        )))
    }

    /// List the models this account can call on this provider, via the
    /// provider's `/models` endpoint (runtime discovery).
    ///
    /// Returns **only the provider's official data** (`RuntimeModel`: id,
    /// owned_by, created) — no community catalogue enrichment. To supplement
    /// with model specs (context length, capabilities, reasoning portrait),
    /// call `get_model_specs` separately and merge in the host (RFC-0027).
    ///
    /// Default returns [`AiMuxError::UnsupportedFunctionality`] — providers that expose a
    /// model-list endpoint override this.
    ///
    /// Implemented as a `Pin<Box<Future>>` (rather than `#[async_trait]`) so that
    /// the dozens of existing `Provider` impls need no changes — only providers
    /// that actually support `/models` override this.
    fn list_models(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<RuntimeModel>, AiMuxError>> + Send + '_>> {
        let name = self.name().to_string();
        Box::pin(async move {
            Err(AiMuxError::UnsupportedFunctionality(format!(
                "list_models not implemented for provider '{name}'"
            )))
        })
    }
}
