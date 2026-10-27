//! Hugging Face provider.
//!
//! [`create_huggingface`] is the Rust form of the AI SDK's `createHuggingFace`:
//! it takes [`HuggingFaceProviderSettings`], validates the base URL, fixes the
//! provider name and returns a [`HuggingFaceProvider`]. The API key is not read
//! there; it is loaded in the request headers of every call, from the setting
//! or from `HUGGINGFACE_API_KEY`. [`huggingface()`] is the default instance.
//!
//! The AI SDK's package serves the Responses API only
//! ([`responses::HuggingFaceResponsesModel`], the lightest Responses
//! implementation: function tools only, no built-in tools), and
//! [`language_model`](aimux_core::provider::Provider::language_model) returns it. There is no Chat
//! Completions model in this package; the router's OpenAI-compatible endpoint
//! is reachable through `create_openai_compatible` with
//! `https://router.huggingface.co/v1`.
//!
//! Requests carry the user-agent of the pinned `@ai-sdk/huggingface@2.0.62`:
//! `ai-sdk-provider-utils/5.0.53`, the provider-utils that version depends
//! on, and no package part, since `createHuggingFace` does not call
//! `withUserAgentSuffix` (huggingface-provider.ts:68-75).

mod error;
mod provider;
pub mod responses;

pub use crate::shared::TransformRequestBody;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{
    HuggingFaceProvider, HuggingFaceProviderSettings, create_huggingface, huggingface,
};
pub use responses::HuggingFaceResponsesModel;

#[cfg(test)]
mod tests;
