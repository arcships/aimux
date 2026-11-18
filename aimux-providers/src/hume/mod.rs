//! Hume speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createHume`
//! (`reference/ai/packages/hume/src/hume-provider.ts`) and `HumeSpeechModel`
//! (`reference/ai/packages/hume/src/hume-speech-model.ts`).
//!
//! Endpoint: `POST https://api.hume.ai/v0/tts/file`
//!
//! Authentication: `X-Hume-Api-Key` header.
//!
//! The Hume TTS API accepts `utterances` (an array of `{ text, voice, speed,
//! description }` objects) and `format` (with a `type` field) in the request
//! body, and returns raw binary audio.
//!
//! `language` is not supported and emits a warning. The model ID is always an
//! empty string (Hume does not use model IDs for TTS).
//!
//! [`create_hume`] takes [`HumeProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`HumeProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `HUME_API_KEY`.
//! [`hume()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod speech_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{HumeProvider, HumeProviderSettings, create_hume, hume};
pub use speech_model::HumeSpeechModel;
