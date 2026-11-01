//! Luma image provider (`@ai-sdk/luma`, `src/index.ts`).
//!
//! Aligned with Vercel AI SDK `LumaImageModel`
//! (`reference/ai/packages/luma/src/luma-image-model.ts`).
//!
//! Uses an async submit + poll pattern: POST to create generation, then GET
//! poll until state is "completed", then download the image.
//!
//! [`create_luma`] takes [`LumaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LumaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LUMA_API_KEY`.
//! [`luma()`] is the default instance; it reads nothing and cannot fail.

mod image_model;
pub(crate) mod options;
mod provider;

pub use image_model::LumaImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{LumaProvider, LumaProviderSettings, create_luma, luma};

#[cfg(test)]
mod tests;
