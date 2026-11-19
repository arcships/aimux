//! Cartesia speech (TTS) and transcription (STT) provider.
//!
//! Aligned with Vercel AI SDK `createCartesia`
//! (`reference/ai/packages/cartesia/src/cartesia-provider.ts`),
//! `CartesiaSpeechModel`
//! (`reference/ai/packages/cartesia/src/cartesia-speech-model.ts`), and
//! `CartesiaTranscriptionModel`
//! (`reference/ai/packages/cartesia/src/cartesia-transcription-model.ts`).
//!
//! Speech endpoint: `POST https://api.cartesia.ai/tts/bytes`
//! Transcription endpoint: `POST https://api.cartesia.ai/stt`
//!
//! Authentication: `Authorization: Bearer {api_key}` header, plus a
//! `Cartesia-Version` header.
//!
//! [`create_cartesia`] takes [`CartesiaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`CartesiaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `CARTESIA_API_KEY`.
//! [`cartesia()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod speech_model;
mod transcription_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{CartesiaProvider, CartesiaProviderSettings, cartesia, create_cartesia};
pub use speech_model::CartesiaSpeechModel;
pub use transcription_model::CartesiaTranscriptionModel;
