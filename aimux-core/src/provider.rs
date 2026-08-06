//! The `Provider` trait — a factory that creates `LanguageModel` instances.

use crate::error::AiMuxError;
use crate::language_model::LanguageModel;

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
        Err(AiMuxError::Unsupported(format!(
            "provider '{}' does not provide language models",
            self.name()
        )))
    }
}
