//! AssemblyAI transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createAssemblyAI` / `AssemblyAITranscriptionModel`
//! (`reference/ai/packages/assemblyai/src/assemblyai-transcription-model.ts`).
//!
//! AssemblyAI uses a three-step async pattern:
//! 1. POST `/v2/upload` with raw audio bytes → returns `upload_url`
//! 2. POST `/v2/transcript` with JSON body (audio_url + options) → returns transcript `id`
//! 3. GET `/v2/transcript/{id}` polling until status is `completed`
//!
//! [`create_assemblyai`] takes [`AssemblyAIProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`AssemblyAIProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `ASSEMBLYAI_API_KEY`.
//! [`assemblyai()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod transcription_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{AssemblyAIProvider, AssemblyAIProviderSettings, assemblyai, create_assemblyai};
pub use transcription_model::AssemblyAITranscriptionModel;
