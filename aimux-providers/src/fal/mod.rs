//! Fal provider (`@ai-sdk/fal`, `src/index.ts`).
//!
//! Aligned with Vercel AI SDK `createFal`, `FalImageModel`,
//! `FalTranscriptionModel`, and `FalVideoModel`
//! (`reference/ai/packages/fal/src/index.ts`).
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

pub use image_model::FalImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{FalProvider, FalProviderSettings, create_fal, fal};
pub use transcription_model::FalTranscriptionModel;
pub use video_model::FalVideoModel;

#[cfg(test)]
mod tests;
