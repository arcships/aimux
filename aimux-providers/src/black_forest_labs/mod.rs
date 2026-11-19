//! Black Forest Labs image provider.
//!
//! Aligned with Vercel AI SDK `BlackForestLabsImageModel`
//! (`reference/ai/packages/black-forest-labs/src/black-forest-labs-image-model.ts`).
//!
//! Uses an async submit + poll pattern: POST to submit, then GET poll_url
//! until status is "Ready", then download the image.
//!
//! [`create_black_forest_labs`] takes [`BlackForestLabsProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`BlackForestLabsProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `BFL_API_KEY`.
//! [`black_forest_labs()`] is the default instance; it reads nothing and cannot fail.

mod image_model;
mod provider;

pub use image_model::BlackForestLabsImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{
    BlackForestLabsProvider, BlackForestLabsProviderSettings, black_forest_labs,
    create_black_forest_labs,
};
