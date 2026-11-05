//! OpenAI-compatible provider.
//!
//! [`create_openai_compatible`] is the Rust form of the AI SDK's
//! `createOpenAICompatible`: it takes [`OpenAICompatibleProviderSettings`],
//! validates the base URL and returns an [`OpenAICompatibleProvider`]
//! whose models speak the chat-completions, embeddings and image endpoints of
//! any OpenAI-compatible server.
//!
//! What is fixed when the provider is created, and what is not:
//!
//! - The explicit settings (`name`, `base_url`, `headers`, `query_params`,
//!   `fetch`, the capability flags, `transform_request_body`) are fixed in the
//!   factory.
//! - A non-empty `api_key` produces a bearer header. `None` or `""` sends
//!   no `Authorization` header, which is what a local server wants.
//!
//! Identity: every model reports `"{name}.{method}"` (`groq.chat`,
//! `local.embedding`, ...). The providerOptions namespace is the same name:
//! options are read from `openaiCompatible`, then the name, then its camelCase
//! form (later wins), fields of the name's own namespace that the generic
//! schema does not know are passed through to the body, and provider metadata
//! is reported under the name.
//!
//! Vendor packages ([`groq`](crate::groq), [`deepseek`](crate::deepseek)) and the
//! registry presets ([`preset`](crate::preset)) are built on the same
//! internals; the shared model never branches on a vendor name.

pub(crate) mod chat;
pub mod completion;
pub mod embedding;
mod error;
pub mod image;
pub(crate) mod internal;
mod provider;
mod request_rewrite;
mod utils;

pub use chat::{
    ConvertUsage, MetadataExtractor, OpenAICompatibleChatModel, StreamMetadataExtractor,
    SupportedUrlsFn,
};
pub use completion::OpenAICompatibleCompletionModel;
pub use embedding::OpenAICompatibleEmbeddingModel;
pub use image::OpenAICompatibleImageModel;
pub use provider::{
    OpenAICompatibleProvider, OpenAICompatibleProviderSettings, create_openai_compatible,
};

pub use crate::shared::TransformRequestBody;

use provider::CompatModelConfig;

// The module tests build compatible models straight from the internals.
#[cfg(test)]
use chat::ChatSettings;
#[cfg(test)]
use provider::{Assembly, BaseUrl, ResolveBaseUrl};

#[cfg(test)]
mod tests;
