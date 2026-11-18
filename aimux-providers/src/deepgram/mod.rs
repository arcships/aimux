//! Deepgram transcription (STT) provider (`@ai-sdk/deepgram`, `src/index.ts`).
//!
//! Aligned with Vercel AI SDK `createDeepgram` / `DeepgramTranscriptionModel`
//! (`reference/ai/packages/deepgram/src/deepgram-transcription-model.ts`).
//!
//! Endpoint: `POST https://api.deepgram.com/v1/listen?{query_params}`
//!
//! The Deepgram API accepts raw audio bytes in the request body (with the
//! `Content-Type` header set to the audio media type) and query parameters for
//! model configuration. It returns a JSON body with `results.channels[0]`
//! containing the transcript, words, and detected language.
//!
//! [`create_deepgram`] takes [`DeepgramProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`DeepgramProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `DEEPGRAM_API_KEY`.
//! [`deepgram()`] is the default instance; it reads nothing and cannot fail.

mod error;
pub(crate) mod options;
mod provider;
mod transcription_model;

#[cfg(test)]
use error::deepgram_error_parts;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{DeepgramProvider, DeepgramProviderSettings, create_deepgram, deepgram};
pub use transcription_model::DeepgramTranscriptionModel;

#[cfg(test)]
mod tests;
