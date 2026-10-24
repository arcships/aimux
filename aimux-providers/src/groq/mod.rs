//! Groq provider.
//!
//! [`create_groq`] is the Rust form of the AI SDK's `createGroq`: it takes
//! [`GroqProviderSettings`], validates the base URL, fixes the provider name
//! and returns a [`GroqProvider`] whose chat models are Groq's own
//! [`GroqChatLanguageModel`] (`"groq.chat"`). The API key is not read there;
//! it is loaded in the request headers of every call, from the setting or
//! from `GROQ_API_KEY`. [`groq()`] is the default instance.

mod api_types;
mod browser_search_models;
mod chat_language_model;
mod chat_language_model_options;
mod convert_to_chat_messages;
mod convert_usage;
mod error;
mod map_finish_reason;
mod prepare_tools;
mod provider;
mod transcription_model;
mod transcription_model_options;

pub use chat_language_model::GroqChatLanguageModel;
#[cfg(test)]
pub(crate) use chat_language_model::chat_test_stream;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{GroqProvider, GroqProviderSettings, create_groq, groq};
pub use transcription_model::GroqTranscriptionModel;

#[cfg(test)]
mod tests;
