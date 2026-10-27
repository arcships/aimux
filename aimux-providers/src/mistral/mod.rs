//! Mistral AI provider.
//!
//! [`create_mistral`] is the Rust form of the AI SDK's `createMistral`: it
//! takes [`MistralProviderSettings`], validates the base URL, fixes the
//! provider name and returns a [`MistralProvider`]. The API key is not read
//! there; it is loaded in the request headers of every call, from the setting
//! or from `MISTRAL_API_KEY`. [`mistral()`] is the default instance.
//!
//! OpenAI-compatible chat completions API with Mistral-specific differences:
//! - Tool choice uses `"any"` instead of `"required"`
//! - Content can be a string or an array of typed parts (text, thinking, image_url)
//! - Usage supports `num_cached_tokens`
//! - Finish reasons include `model_length`

mod api_types;
mod chat_language_model;
mod chat_language_model_options;
mod convert_to_chat_messages;
mod convert_usage;
mod embedding_model;
mod embedding_model_options;
mod error;
mod map_finish_reason;
mod prepare_tools;
mod provider;
mod speech_model;
mod speech_model_options;
mod transcription_model;
mod transcription_model_options;

pub use chat_language_model::MistralModel;
#[cfg(test)]
pub(crate) use chat_language_model::chat_test_stream;
pub use embedding_model::MistralEmbeddingModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{MistralProvider, MistralProviderSettings, create_mistral, mistral};
pub use speech_model::MistralSpeechModel;
pub use transcription_model::MistralTranscriptionModel;

#[cfg(test)]
mod tests;
