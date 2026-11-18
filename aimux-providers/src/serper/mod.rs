//! Serper provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Serper search API
//! (`POST https://google.serper.dev/search`). Uses `X-API-KEY` header auth
//! via `SERPER_API_KEY`.
//!
//! [`create_serper`] takes [`SerperProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`SerperProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `SERPER_API_KEY`.
//! [`serper()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{SerperProvider, SerperProviderSettings, create_serper, serper};
pub use search_model::SerperSearchModel;
