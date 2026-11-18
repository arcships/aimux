//! LMNT speech (TTS) provider.
//!
//! Aligned with Vercel AI SDK `createLMNT`
//! (`reference/ai/packages/lmnt/src/lmnt-provider.ts`) and `LMNTSpeechModel`
//! (`reference/ai/packages/lmnt/src/lmnt-speech-model.ts`).
//!
//! Endpoint: `POST https://api.lmnt.com/v1/ai/speech/bytes`
//!
//! Authentication: `x-api-key` header.
//!
//! The LMNT TTS API accepts `model`, `text`, `voice`, `response_format`,
//! `speed`, and `language` in the request body, and returns raw binary audio.
//! Provider options (`lmnt` key) support `conversational`, `length`, `seed`,
//! `speed`, `temperature`, `topP`, `sampleRate`, and `format`.
//!
//! [`create_lmnt`] takes [`LMNTProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LMNTProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LMNT_API_KEY`.
//! [`lmnt()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod speech_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{LMNTProvider, LMNTProviderSettings, create_lmnt, lmnt};
pub use speech_model::LMNTSpeechModel;
