//! Recraft image provider.
//!
//! OpenAI Images-compatible API with Recraft extension fields
//! (`style`, `style_id`, `negative_prompt`, `random_seed`).
//!
//! - Endpoint: `POST {base_url}/images/generations` (JSON body)
//! - Auth: Bearer token (`RECRAFT_API_TOKEN`)
//! - Base URL: `https://external.api.recraft.ai/v1`
//! - Response: `{ "data": [{ "url" | "b64_json" }] }` (OpenAI Images shape)
//! - No streaming.
//!
//! [`create_recraft`] takes [`RecraftProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`RecraftProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `RECRAFT_API_TOKEN`.
//! [`recraft()`] is the default instance; it reads nothing and cannot fail.

mod image_model;
mod provider;

pub use image_model::RecraftImageModel;
pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{RecraftProvider, RecraftProviderSettings, create_recraft, recraft};
