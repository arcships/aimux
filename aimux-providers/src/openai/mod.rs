//! OpenAI provider (`@ai-sdk/openai`). One file per upstream file, named after
//! it without the `openai-` prefix, in the upstream sub-directories; this file
//! is `index.ts`.
//!
//! [`create_openai`] is the Rust form of the AI SDK's `createOpenAI`: it takes
//! [`OpenAIProviderSettings`], validates the base URL, fixes the provider name
//! and returns an [`OpenAIProvider`]. The API key is not read there; it is
//! loaded in the request headers of every call, from the setting or from
//! `OPENAI_API_KEY`. [`openai()`] is the default instance.
//!
//! This package is the native OpenAI API only. Servers that merely speak the
//! same wire format are served by [`crate::openai_compatible`] (and the
//! registry presets built on it).

mod chat;
pub mod completion;
pub(crate) mod config;
pub mod decision;
pub mod embedding;
mod error;
pub mod files;
pub mod image;
mod language_model_capabilities;
mod normalize_json_schema;
mod provider;
pub mod responses;
pub mod speech;
mod stream_error;
mod tool;
pub mod tools;
pub mod transcription;

pub use chat::OpenAIModel;
pub use completion::OpenAICompletionModel;
pub use decision::OpenAIDecisionModel;
pub use embedding::OpenAIEmbeddingModel;
pub use files::OpenAIFiles;
pub use image::OpenAIImageModel;
pub use provider::{OpenAIProvider, OpenAIProviderSettings, create_openai, openai};
pub use responses::OpenAIResponsesModel;
pub use speech::OpenAISpeechModel;
pub use tools::{OpenAIComputerAction, OpenAIComputerSafetyCheck};
pub use transcription::OpenAITranscriptionModel;

// Paths the other vendors (Azure, the OpenAI-compatible wrappers, the default
// registry) import.
pub(crate) use error::openai_failed_response_handler;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub(crate) use stream_error::openai_stream_error;

/// The chat helpers the other vendors' converters reuse.
pub(crate) mod convert {
    pub(crate) use super::chat::chat_language_model_options::parse_option_fields;
}

/// The model listing and the chat stream test driver the other vendors reuse.
pub(crate) mod model {
    #[cfg(test)]
    pub(crate) use super::chat::chat_language_model::chat_test_stream;
    pub(crate) use super::provider::list_models_once;
}

/// The chat options parser Azure reuses.
pub(crate) mod options {
    pub(crate) use super::chat::chat_language_model_options::{
        ChatOptionsParser, parse_chat_options,
    };
}

#[cfg(test)]
mod tests;
