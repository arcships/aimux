//! The text-completion model (`src/completion/`).

mod completion_language_model;
mod convert_completion_usage;
mod convert_to_completion_prompt;
mod get_response_metadata;

pub use completion_language_model::OpenAICompatibleCompletionModel;
