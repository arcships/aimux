//! fal image, transcription (STT) and video provider (`@ai-sdk/fal`,
//! `src/index.ts`).
//!
//! Aligned with Vercel AI SDK `createFal` / `FalTranscriptionModel`
//! (`reference/ai/packages/fal/src/fal-transcription-model.ts`).
//!
//! Fal uses an async queue pattern: POST to submit, then GET to poll until
//! the result is ready. The audio is sent as a base64 data URL.
//!
//! [`create_fal`] takes [`FalProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`FalProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `FAL_KEY`.
//! [`fal()`] is the default instance; it reads nothing and cannot fail.

mod error;
mod image_model;
pub(crate) mod options;
mod provider;
mod transcription_model;
mod video_model;

#[cfg(test)]
use error::fal_error_parts;
pub use image_model::FalImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{FalProvider, FalProviderSettings, create_fal, fal};
pub use transcription_model::FalTranscriptionModel;
pub use video_model::FalVideoModel;

#[cfg(test)]
mod tests;
