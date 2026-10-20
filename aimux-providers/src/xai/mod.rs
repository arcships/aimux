//! xAI (Grok) provider.
//!
//! [`create_xai`] is the Rust form of the AI SDK's `createXai`: it takes
//! [`XAIProviderSettings`], removes a trailing slash, fixes the provider name and
//! returns an [`XAIProvider`]. The API key is not read there; it is loaded in
//! the request headers of every call, from the setting or from
//! `XAI_API_KEY`. [`xai()`] is the default instance.
//!
//! The AI SDK's xAI package serves text through the Responses API only
//! ([`XaiResponsesModel`], `provider()` = `xai.responses`), and
//! [`language_model`](aimux_core::provider::Provider::language_model) returns it. There is no Chat
//! Completions model in this package; xAI's OpenAI-compatible endpoint is
//! reachable through `create_openai_compatible` with `https://api.x.ai/v1`.
//!
//! The package also has speech ([`XaiSpeechModel`]), transcription
//! ([`XaiTranscriptionModel`], HTTP and WebSocket streaming) and video
//! ([`XaiVideoModel`]) models. Speech and transcription take no model id.

mod error;
pub(crate) mod options;
mod provider;
pub mod responses;
mod speech_model;
mod supports_reasoning_effort;
mod transcription_model;
mod video_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{XAIProvider, XAIProviderSettings, create_xai, xai};
pub use responses::XaiResponsesModel;
pub use speech_model::XaiSpeechModel;
pub use supports_reasoning_effort::supports_reasoning_effort;
pub use transcription_model::XaiTranscriptionModel;
pub use video_model::XaiVideoModel;

#[cfg(test)]
mod tests;
