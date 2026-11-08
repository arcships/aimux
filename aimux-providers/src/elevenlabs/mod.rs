//! ElevenLabs speech (TTS) and transcription (STT) provider
//! (`@ai-sdk/elevenlabs`, `src/index.ts`).
//!
//! Aligned with Vercel AI SDK `createElevenLabs`
//! (`reference/ai/packages/elevenlabs/src/elevenlabs-provider.ts`) and
//! `ElevenLabsSpeechModel`
//! (`reference/ai/packages/elevenlabs/src/elevenlabs-speech-model.ts`).
//!
//! Endpoint: `POST https://api.elevenlabs.io/v1/text-to-speech/{voiceId}`
//!
//! Authentication: `xi-api-key` header.
//!
//! The ElevenLabs TTS API accepts `text` and `model_id` in the request body and
//! returns raw binary audio. The `output_format` is passed as a query parameter.
//! `instructions` is not supported and emits a warning.

mod error;
pub(crate) mod options;
mod provider;
mod speech_model;
mod transcription_model;

#[cfg(test)]
use error::elevenlabs_error_parts;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{ElevenLabsProvider, ElevenLabsProviderSettings, create_elevenlabs, elevenlabs};
pub use speech_model::ElevenLabsSpeechModel;
pub use transcription_model::ElevenLabsTranscriptionModel;

#[cfg(test)]
mod tests;
