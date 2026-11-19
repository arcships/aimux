//! KlingAI video generation provider.
//!
//! Aligned with Vercel AI SDK `createKlingAI` / `KlingAIVideoModel`
//! (`reference/ai/packages/klingai/src/klingai-video-model.ts`).
//!
//! KlingAI uses an async task pattern:
//! 1. POST to `/v1/videos/text2video` (or `image2video`) → returns task `id` + `task_id`
//! 2. GET `/v1/videos/text2video/{id}/{task_id}` — polled by Core via `do_status`
//!    until `succeeded`
//! 3. Return the video URL from the result
//!
//! [`create_klingai`] takes [`KlingAIProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`KlingAIProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `KLINGAI_API_KEY`.
//! [`klingai()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod video_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{KlingAIProvider, KlingAIProviderSettings, create_klingai, klingai};
pub use video_model::KlingAIVideoModel;
