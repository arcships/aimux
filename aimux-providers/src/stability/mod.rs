//! Stability image provider (image modality only).
//!
//! Aligned with the Stability AI Stable Image generate API
//! (`https://api.stability.ai/v2beta/stable-image/generate/{ultra|core|sd3}`).
//!
//! Text-to-image generation via `multipart/form-data`. The model ID selects the
//! endpoint sub-path: `stable-image-ultra` → `/generate/ultra`,
//! `stable-image-core` → `/generate/core`, `sd3` → `/generate/sd3`. Any other
//! model ID is used as-is for the sub-path. Responses are image binary
//! (`Accept: image/*`) or base64-encoded JSON.
//!
//! [`create_stability`] takes [`StabilityProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`StabilityProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `STABILITY_API_KEY`.
//! [`stability()`] is the default instance; it reads nothing and cannot fail.

mod image_model;
mod provider;

pub use image_model::StabilityImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{StabilityProvider, StabilityProviderSettings, create_stability, stability};
