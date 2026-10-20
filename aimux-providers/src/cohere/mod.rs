//! Cohere provider.
//!
//! [`create_cohere`] is the Rust form of the AI SDK's `createCohere`: it takes
//! [`CohereProviderSettings`], normalizes the base URL, fixes the provider name
//! and returns a [`CohereProvider`]. The API key is not read there; it is
//! loaded in the request headers of every call, from the setting or from
//! `COHERE_API_KEY`. [`cohere()`] is the default instance.
//!
//! Implements Cohere's v2 chat API with its own message format (not
//! OpenAI-compatible). Supports text generation, streaming, tool calls,
//! and reasoning (thinking).

mod api_types;
mod chat_language_model;
mod chat_language_model_options;
mod convert_to_chat_prompt;
mod convert_usage;
mod embedding_model;
mod embedding_model_options;
mod error;
mod map_finish_reason;
mod prepare_tools;
mod provider;
mod reranking;

pub use chat_language_model::CohereModel;
pub use embedding_model::CohereEmbeddingModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{CohereProvider, CohereProviderSettings, cohere, create_cohere};
pub use reranking::CohereRerankingModel;
