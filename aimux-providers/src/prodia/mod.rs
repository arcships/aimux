//! Prodia image and video provider.
//!
//! Aligned with Vercel AI SDK `ProdiaImageModel`
//! (`reference/ai/packages/prodia/src/prodia-image-model.ts`) and
//! `ProdiaVideoModel`
//! (`reference/ai/packages/prodia/src/prodia-video-model.ts`).
//!
//! POST to `/job?price=true`, returns a multipart response with a JSON "job"
//! part and a binary "output" image part.
//!
//! [`create_prodia`] takes [`ProdiaProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ProdiaProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `PRODIA_API_KEY`.
//! [`prodia()`] is the default instance; it reads nothing and cannot fail.

mod image_model;
mod provider;
mod video_model;

pub use image_model::ProdiaImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{ProdiaProvider, ProdiaProviderSettings, create_prodia, prodia};
pub use video_model::ProdiaVideoModel;
