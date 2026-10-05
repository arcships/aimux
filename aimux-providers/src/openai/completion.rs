//! Native text-completion language model.

/// The native provider's `/completions` model.
pub type OpenAICompletionModel =
    crate::openai_compatible::completion::OpenAICompatibleCompletionModel;
