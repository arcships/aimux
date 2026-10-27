//! The text-completion model (`src/completion/`).

mod completion_language_model;
mod convert_completion_usage;
mod convert_to_completion_prompt;
mod get_response_metadata;

#[cfg(test)]
pub(in crate::openai_compatible) use completion_language_model::CompletionConfig;
pub use completion_language_model::OpenAICompatibleCompletionModel;
