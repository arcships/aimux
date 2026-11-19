//! SearXNG provider — search modality only.
//!
//! Implements the `SearchModel` trait against a self-hosted SearXNG instance
//! (`GET {SEARXNG_URL}/search?q=...&format=json`).
//!
//! SearXNG is a self-hosted, normally unauthenticated provider: there is no
//! default instance. The instance URL is the `base_url` setting or the
//! `SEARXNG_URL` environment variable, read when a request is made (a missing
//! URL fails that request with `AiMuxError::LoadSetting`). A `403` response
//! typically indicates that the `json` output format is not enabled on the
//! instance.
//!
//! [`create_searxng`] takes [`SearxngProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`SearxngProvider`]. The instance URL is not read
//! there: the instance URL is resolved for every request.
//! [`searxng()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub use provider::{SearxngProvider, SearxngProviderSettings, create_searxng, searxng};
pub use search_model::SearxngSearchModel;
