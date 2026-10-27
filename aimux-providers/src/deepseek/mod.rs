//! DeepSeek provider.
//!
//! [`create_deepseek`] is the Rust form of the AI SDK's `createDeepSeek`: a
//! provider of [`DeepSeekChatLanguageModel`], the package's own chat model
//! (`@ai-sdk/deepseek` does not build on the OpenAI-compatible package). Its
//! models report `"deepseek.chat"` and read providerOptions and report provider
//! metadata under `deepseek`.
//!
//! As in the other packages the API key is not read when the provider is
//! created: `DEEPSEEK_API_KEY` is loaded on every request unless `api_key` is
//! given. [`deepseek()`] is the default instance.

mod chat;
mod provider;

pub use chat::DeepSeekChatLanguageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{DeepSeekProvider, DeepSeekProviderSettings, create_deepseek, deepseek};

#[cfg(test)]
mod tests;
