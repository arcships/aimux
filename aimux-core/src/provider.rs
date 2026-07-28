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
    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError>;
}
