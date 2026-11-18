//! Rev.ai transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createRevai` / `RevaiTranscriptionModel`
//! (`reference/ai/packages/revai/src/revai-transcription-model.ts`).
//!
//! Rev.ai uses an async job pattern:
//! 1. POST `/speechtotext/v1/jobs` with multipart form (media file + config JSON)
//! 2. GET `/speechtotext/v1/jobs/{id}` to poll until status is `transcribed`
//! 3. GET `/speechtotext/v1/jobs/{id}/transcript` to fetch the final transcript
//!
//! [`create_revai`] takes [`RevaiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`RevaiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `REVAI_API_KEY`.
//! [`revai()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod transcription_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{RevaiProvider, RevaiProviderSettings, create_revai, revai};
pub use transcription_model::RevaiTranscriptionModel;
