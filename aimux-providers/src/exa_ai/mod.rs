//! Exa AI provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Exa search API
//! (`POST https://api.exa.ai/search`). Uses `x-api-key` header auth
//! via `EXA_API_KEY`.
//!
//! [`create_exa_ai`] takes [`ExaAiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ExaAiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `EXA_API_KEY`.
//! [`exa_ai()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{ExaAiProvider, ExaAiProviderSettings, create_exa_ai, exa_ai};
pub use search_model::ExaAiSearchModel;
