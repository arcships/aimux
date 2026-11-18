//! Gladia transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createGladia` / `GladiaTranscriptionModel`
//! (`reference/ai/packages/gladia/src/gladia-transcription-model.ts`).
//!
//! Gladia uses a three-step async pattern:
//! 1. POST `/v2/upload` with multipart form (audio file) → returns `audio_url`
//! 2. POST `/v2/pre-recorded` with JSON body (audio_url + options) → returns `result_url`
//! 3. GET `result_url` polling until status is `done`
//!
//! [`create_gladia`] takes [`GladiaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`GladiaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `GLADIA_API_KEY`.
//! [`gladia()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod transcription_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{GladiaProvider, GladiaProviderSettings, create_gladia, gladia};
pub use transcription_model::GladiaTranscriptionModel;
