//! RunwayML video generation provider (video modality only).
//!
//! RunwayML uses an async task pattern:
//! 1. POST to `/v1/text_to_video` (or `/v1/image_to_video` when an input image
//!    is supplied) → returns the task `id`.
//! 2. GET `/v1/tasks/{id}` polled until `SUCCEEDED` or `FAILED`.
//! 3. Return the generated video URL(s) from the task `output` array.
//!
//! Authentication is a `Bearer` token and every request must carry the
//! `X-Runway-Version: 2024-11-06` header.
//!
//! [`create_runwayml`] takes [`RunwaymlProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`RunwaymlProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `RUNWAYML_API_SECRET`.
//! [`runwayml()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod video_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{RunwaymlProvider, RunwaymlProviderSettings, create_runwayml, runwayml};
pub use video_model::RunwaymlVideoModel;
