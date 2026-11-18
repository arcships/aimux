//! Parallel AI provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Parallel AI search API
//! (`POST https://api.parallel.ai/v1/search`).
//!
//! Parallel AI is a modality-specific provider: it exposes a native web
//! search protocol (objective + search queries → results) and does not
//! support language models. The single `query` is mapped to the
//! `search_queries` array (and used as the `objective`).
//!
//! [`create_parallel_ai`] takes [`ParallelAiProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`ParallelAiProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `PARALLEL_API_KEY`.
//! [`parallel_ai()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{
    ParallelAiProvider, ParallelAiProviderSettings, create_parallel_ai, parallel_ai,
};
pub use search_model::ParallelAiSearchModel;
