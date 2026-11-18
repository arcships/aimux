//! TinyFish search provider — implements the `SearchModel` trait.
//!
//! Implements the TinyFish search API
//! (`GET https://api.search.tinyfish.ai?query=...&count=...`).
//!
//! Authentication uses the `X-API-Key` header (env `TINYFISH_API_KEY`).
//! TinyFish is a search-only provider: it exposes a web search protocol
//! (query → results) and does not support language models.
//!
//! [`create_tinyfish`] takes [`TinyfishProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`TinyfishProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `TINYFISH_API_KEY`.
//! [`tinyfish()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{TinyfishProvider, TinyfishProviderSettings, create_tinyfish, tinyfish};
pub use search_model::TinyfishSearchModel;
