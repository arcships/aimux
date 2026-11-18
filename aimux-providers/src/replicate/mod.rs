//! Replicate image and video provider.
//!
//! Aligned with Vercel AI SDK `ReplicateImageModel`
//! (`reference/ai/packages/replicate/src/replicate-image-model.ts`) and
//! `ReplicateVideoModel`
//! (`reference/ai/packages/replicate/src/replicate-video-model.ts`).
//!
//! [`create_replicate`] takes [`ReplicateProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ReplicateProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `REPLICATE_API_TOKEN`.
//! [`replicate()`] is the default instance; it reads nothing and cannot fail.

mod image_model;
mod provider;
mod video_model;

pub use image_model::ReplicateImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{ReplicateProvider, ReplicateProviderSettings, create_replicate, replicate};
pub use video_model::ReplicateVideoModel;
