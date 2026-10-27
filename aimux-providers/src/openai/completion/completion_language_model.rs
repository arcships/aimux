//! Native text-completion language model (openai-completion-language-model.ts),
//! served by the OpenAI-compatible completion model.

/// The native provider's `/completions` model.
pub type OpenAICompletionModel =
    crate::openai_compatible::completion::OpenAICompatibleCompletionModel;
