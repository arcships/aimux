//! Anthropic provider.
//!
//! [`create_anthropic`] is the Rust form of the AI SDK's `createAnthropic`: it
//! takes [`AnthropicProviderSettings`], validates the base URL, rejects an
//! `api_key` given together with an `auth_token`, fixes the provider name and
//! returns an [`AnthropicProvider`]. Credentials are not read there; they are
//! loaded in the request headers of every call, from the settings or (for the
//! key) from `ANTHROPIC_API_KEY`. [`anthropic()`] is the default instance.
//!
//! The Messages model is shared with Anthropic on AWS
//! ([`crate::anthropic_aws`]) and Anthropic on Vertex
//! ([`crate::vertex`]): they build the same private model configuration and differ
//! only in what it holds.

mod api;
pub(crate) mod config;
mod convert_to_prompt;
mod convert_usage;
mod error;
pub mod files;
mod get_cache_control;
mod language_model;
pub(crate) mod language_model_options;
mod map_stop_reason;
mod message_metadata;
pub mod prepare_tools;
mod provider;
pub(crate) mod request_rewrite;
pub mod sanitize_json_schema;
pub mod skills;

pub use files::AnthropicFiles;
pub use language_model::AnthropicMessagesModel;
pub use provider::{AnthropicProvider, AnthropicProviderSettings, anthropic, create_anthropic};
pub use skills::AnthropicSkills;

pub(crate) use error::anthropic_failed_response_handler;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL, list_models_once, supported_urls};

/// The options module under the path Anthropic AWS and Anthropic on Vertex
/// import it from.
pub(crate) use language_model_options as options;

#[cfg(test)]
mod tests;
